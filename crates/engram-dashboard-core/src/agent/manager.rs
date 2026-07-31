//! AgentManager — Phase 1 결합부. backend/transport/output_core/session을 묶어 에이전트
//! 생명주기를 관리한다. S10: PtyManager→AgentManager 개명 + 신경로 전환.
//! S9: 프로필 기반 spawn + 세션 복원(restore_all) + claude 세션 추적 부착(불변).
//!
//! 신경로(S10): manager는 backend(CommandSpec 산출) → PtyTransport(자원) +
//! OutputCore(출력) → AgentSession(합성)을 조립한다. 옛 PtySession/drain.rs/claude.rs는 제거됨.
//!
//! tauri import 0 — 상위 상태 알림은 StatusSink trait으로 주입받는다(AppHandle 아님).
//!
//! ★명부(roster) 단일 소유자(ADR-0119)★: "전체 에이전트 + 각자 살아있음/잠듦" 은 `roster()` 한 곳에서만
//! 만들어진다. 프로필 레지스트리는 이 타입 **안**에 있고 밖으로 핸들이 나가지 않는다(옛 `profiles()`
//! 접근자 제거) — 바깥은 좁은 동사(create/delete/rename/reparent/set-auto-restore/snapshot)만 쓴다.
//! canonical 이름은 명부 전체에서 유일하며(ADR-0120), 강제 지점은 생성·신규 등록(spawn)·개명 셋뿐이다.
//!
//! 락 순서(LLD §10 규칙1): `sessions` RwLock은 조회 전용이다. Arc<AgentSession>을 clone하고
//! lock을 즉시 해제한 뒤에야 session 내부 lock(core/transport)을 취득한다. sessions lock
//! 보유 중 session 내부 lock 취득은 금지(데드락 방지).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::agent::backend;
use crate::agent::backend::InputEncoder;
use crate::agent::output_core::OutputCore;
use crate::agent::preset::PresetRegistry;
use crate::agent::profile::{
    AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
};
use crate::agent::reaper::{self, ReaperCmd, ReaperDeps};
use crate::agent::session::AgentSession;
use crate::agent::session_tracker::SessionTracker;
use crate::agent::transport::pty::PtyTransport;
use crate::agent::transport::stdio::StdioTransport;
use crate::agent::transport::{AgentTransport, OutputDecoder};
use crate::agent::types::{
    AgentId, AgentInfo, AgentStatus, BackendCaps, CommandSpec, ControlChannel, NoopControlChannel,
    OutputChunk, OutputEvent, OutputSink, PtyError, ReapMsg, SinkId, StatusSink, SubscribeOutcome,
    TerminalReason, TerminationIntent,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// resume spawn 후 이 시간 안에 비정상 종료(code≠0/Failed/Killed)하면 resume 실패로 판정한다
/// (H-1.7 "조기 종료 윈도"). 성공한 resume은 TUI라 계속 떠 있다.
/// ★ADR-0082★: 옛날엔 이 신호를 fresh-fallback(새 대화 자동 생성)으로 번역했으나, 이제는
/// **Failed(시체) 종점 + 원인 로그**로 번역한다 — 자동으로 새 대화를 만들지 않는다.
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);
/// 복원 시 에이전트 간 spawn 간격(동시 폭주 방지 stagger).
const RESTORE_STAGGER: Duration = Duration::from_millis(200);

/// 검증·기본용 셸. 프로필 없이 빠르게 띄울 때 commands가 사용한다.
#[cfg(windows)]
pub fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
pub fn default_shell() -> &'static str {
    "bash"
}

/// 모드에 따라 transport를 고른다(ADR-0044 SEAM 1 조립 분기). json 모드 = StdioTransport(파이프,
/// 구조화 출력 캐리어), 그 외(터미널·shell) = PtyTransport(ConPTY, 기존 동작 불변).
///
/// 조립 규칙(양 끝 3지점 중 ①): backend가 mode 보고 CommandSpec 인자를 구성하고, manager는 여기서
/// transport 종류를 고른다. transport 자체는 claude/json을 모른다 — spec만 받아 프로세스를 띄운다.
/// 반환: (박싱된 transport, child_pid). 별도 함수로 뺀 이유 = 실 claude 없이 선택 로직을 단위
/// 테스트하기 위함(ADR-0012 격리 — json→structured caps / 터미널→아님).
///
/// ★조립점 — "mode → 통로가 나르는 것"의 단일 위치(FIX 2, 사용자 요청: 한 곳에 모음)★:
///   transport 종류 선택뿐 아니라 **출력이 구조화(NDJSON)인지도 여기서 결정해 주입**한다. 파이프
///   자체는 내용을 모르므로(통로 무정제 불변) StdioTransport 는 structured 를 하드코딩하지 않고
///   이 지점의 주입값을 받아 caps 로 신고한다. json 모드 = claude `--output-format stream-json` →
///   NDJSON 캐리어 → structured=true. 터미널(PtyTransport)은 그 자체로 terminal-bytes(구조화 아님).
///   출처 분리(output=transport 소유, ADR-0030)는 유지 — 값만 이 조립점에서 주입한다.
// ADR-0044
// ADR-0030
fn select_transport(
    json_mode: bool,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
    decoder: Option<Box<dyn OutputDecoder>>,
) -> Result<(Box<dyn AgentTransport>, Option<u32>), PtyError> {
    if json_mode {
        // json 모드: PTY 없는 파이프. cols/rows는 파이프에 개념 없어 무시.
        // structured=true 주입 — json 모드가 곧 NDJSON 캐리어라는 mode→caps 매핑(위 조립점 규칙).
        // ★decoder 주입(ADR-0004)★: backend 가 만든 출력 정제기를 통로에 꽂는다 — StdioTransport 는
        //   이게 어떤 디코더인지 모른 채 pump 에서 적용만 한다(통로는 claude 를 모름).
        let (t, pid) = StdioTransport::open(spec, true, decoder)?;
        Ok((Box::new(t), pid))
    } else {
        // 터미널·shell = PtyTransport. decoder 는 여기 경로에선 항상 None(직통) — 방어적으로 무시.
        // (backend::output_decoder 가 json 모드에만 Some 을 주므로 non-json 은 애초에 None 이 온다.)
        let (t, pid) = PtyTransport::open(spec, cols, rows)?;
        Ok((Box::new(t), pid))
    }
}

/// 명부(roster) 항목 하나 = **에이전트 하나**(ADR-0119 결정 1). 살아 있으면 세션이 붙고(`live=Some`)
/// 잠들어 있으면 안 붙는다(`live=None`). "산 목록"과 "프로필 목록"을 소비자가 각자 합치던 중복을
/// 없애는 것이 이 타입의 존재 이유다 — 합성은 `AgentManager::roster()` 한 곳에서만 일어난다.
///
/// ★`canonical_name` 의 출처는 생사에 따라 다르다(의도)★ — 산 항목은 `AgentInfo.name`, 잠든 항목은
///   `AgentProfile::canonical_name_when_live()`. 근거는 `roster()` doc(합치면 파킹 키가 흔들린다).
/// ★터미널 상태로 맵에 남은 시체는 항목이 아니다★ — 산 것도 잠든 것도 아니다. 프로필이 남아 있으면
///   잠듦으로, 없으면(ad-hoc) 아예 목록에 없다. 이건 `addressing_sources` 의 기존 동작 그대로다.
// ADR-0119
#[derive(Debug, Clone)]
pub struct RosterEntry {
    pub id: AgentId,
    pub canonical_name: String,
    /// `Some` = 살아 있음(Running|Exiting 세션 부착) · `None` = 잠듦(프로필만 있음).
    pub live: Option<AgentInfo>,
}

/// 저장 직전의 표시명 override 정규화 — 양끝 공백만 깎고, 남는 게 없으면 override 없음(`None`)으로 본다.
/// 이름 배정 게이트를 여는 셋(`create_agent`·`rename_agent`·`register_for_spawn`)이 전부 통과한다. 다만
/// **지금 실제로 override 를 싣는 경로는 개명 하나뿐**이다 — 표시명 override 를 나르는 wire 명령은
/// `RenameProfile` 뿐이고, 생성·spawn 쪽에는 그 필드가 아예 없다(`CreateProfile.name` 은 `AgentProfile.name`
/// 이고 `AgentProfile::new` 가 `display_name: None` 을 박는다). 그래서 나머지 둘은 공개 API 방어선이다.
///
/// ★저장된 이름 · 화면에 그려지는 이름 · 편지 주소가 **같은 문자열**이어야 한다★: 유일성(ADR-0120) 판정은
///   문자열 비교라 `bob` 과 `" bob "` 은 서로 다른 이름으로 **둘 다** 통과하는데 트리에는 똑같이 그려진다 —
///   사용자는 편지가 둘 중 누구에게 가는지 구분할 수 없다. 게다가 메시징 입구가 수신자 토큰을 trim 해서
///   맞추므로 `" bob "` 으로 저장된 에이전트는 보이면서도 이름으로 주소 지정이 안 된다.
/// ★그 입구 trim(`messaging` service 수신자 대조)은 지우지 말 것★: **CLI 입구**(`engram-send --to a,b`)가
///   셸 제약 때문에 수신자 목록을 콤마로 쪼개는데 그때 공백을 떼지 않아(`"alice, bob"` → `["alice", " bob"]`)
///   두 번째 이후 수신자를 그 trim 이 구제한다. MCP 입구는 배열 원소를 **절대 쪼개지 않으므로**(spec §6 —
///   쪼개면 `"a,b"` 라는 실제 이름을 표현할 수 없다) 그쪽만 보고 "쪼개는 데가 없으니 trim 도 불필요" 라고
///   결론내면 콤마 목록이 깨진다. 저장을 정규화하면 그 trim 은 잘 저장된 이름에 대해 no-op 이 될 뿐이고,
///   콤마 목록 구제 역할은 그대로 남는다.
/// ★유일성 판정 **전에** 건다★: 판정과 저장이 같은 정규화 값을 봐야 `" bob "` 이 모든 면에서 `bob` 요청이
///   된다(뒤에 걸면 `" bob "` 이 빈 이름으로 판정돼 동명이 다시 새어 들어온다).
/// ★안쪽 공백은 이름의 일부다★ — `"bob smith"` 는 그대로 살아야 하므로 양끝만 깎는다.
/// ★남는 문제는 "같은 구멍의 잔여" 가 아니라 다른 종류다★: `str::trim` 은 Unicode White_Space(스페이스·탭·
///   NBSP)만 걷어내고 zero-width(U+200B 등)는 **양쪽 어디서도** 떨어지지 않는다 — 그래서 그런 이름은
///   저장 == 표시 == 주소가 그대로 성립해 위 불변식을 깨지 않는다(패딩 이름이 깬 것은 *주소 도달성*이었다).
///   남는 것은 눈으로 구분이 안 되는 **시각적 혼동**뿐이고, 그 해법(NFKC·confusable folding)은 정당한
///   이름까지 뭉개므로 여기서 즉흥 필터로 처리하지 않는다 — 정책 결정 사항이다.
/// ★이미 저장된 이름은 고치지 않는다★ — 지금부터의 쓰기에만 걸린다(마이그레이션 장치 없음).
// ADR-0120
fn normalize_display_name(display_name: Option<String>) -> Option<String> {
    let trimmed = display_name?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// `name` 이 `base` 계열일 때의 형태. ★`Exact` 와 `Suffixed(0)` 를 **절대 한 값으로 섞지 않는다**★:
/// 섞으면 리터럴 `bob(0)` 하나가 접미사 없는 `bob` 을 점유한 것처럼 보여, `bob` 이 비어 있는데도
/// 다음 요청이 `bob(1)` 을 받는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameKind {
    /// `name == base` — 접미사가 아예 없다(= 그 이름 자체가 쓰이고 있음).
    Exact,
    /// `name == base(n)` — 리터럴 `base(0)` 도 여기 온다.
    Suffixed(u32),
}

/// `name` 이 `base` 계열인지 분류한다. ADR-0115 표기(`이름(N)`)의 파서로, 발급기
/// (`assign_unique_name`)의 `format!("{base}({n})")` 와 **정확한 역함수**여야 한다 — 표기를 바꾸면 둘 다 바꾼다.
fn classify_name(base: &str, name: &str) -> Option<NameKind> {
    if name == base {
        return Some(NameKind::Exact);
    }
    let rest = name.strip_prefix(base)?;
    let digits = rest.strip_prefix('(')?.strip_suffix(')')?;
    // 빈 괄호·부호·공백은 우리 표기가 아니다(`base()`·`base(-1)` 은 남남).
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // 선행 0(`base(01)`)도 우리 표기가 아니다 — 발급기는 절대 그렇게 쓰지 않는다. 계열에서 빼도
    // 중복이 생기지 않는다: `base(01)` 은 발급 가능한 어떤 문자열(`base(1)` 등)과도 다른 문자열이다.
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    digits.parse::<u32>().ok().map(NameKind::Suffixed)
}

/// 접미사 공간 소진의 단일 에러 문구 — 유일성을 포기하고 중복 이름을 발급하는 대신 거부한다(이름 =
/// 주소이므로 중복은 메일 오배달로 번역된다, ADR-0116).
///
/// ★현실 도달 불가★: `pick_suffix` 는 계열의 1..=u32::MAX 가 **전부** 점유됐을 때만 `None` 을 내므로
/// (명부에 42억 엔트리) 이 거부는 실제로 발화하지 않는다. 포화는 그 전에 "가장 낮은 빈 번호" 로 흡수된다.
/// 그래도 이 경로와 `RenameOutcome::Exhausted` 를 남기는 이유는 결정표를 전역 함수로 닫아 두는 것뿐이다 —
/// 살아 있는 정책으로 읽지 말 것.
/// ★전용 에러 변형을 만들지 않는다★: 호출부에 필요한 사실은 "이 동사를 지금 수행할 수 없었다" 하나이고
/// 그건 이미 있는 미지원 신호와 같은 모양이다(`write_stdin_observed_if_epoch` 와 동일 판단).
fn name_space_exhausted(base: &str) -> PtyError {
    PtyError::Unsupported(format!(
        "name suffix space exhausted for base {base:?} — refusing rather than minting a duplicate name"
    ))
}

/// 이름 결정표의 결말 하나(ADR-0120 유일성 · ADR-0123 번호 규칙). 배정 게이트 보유 중에 산출된다.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NameDecision {
    /// 요청 이름을 아무도 갖고 있지 않다 → 접미사 없이 그대로 확정.
    Free,
    /// 요청 이름은 남에게 있고, 자기가 이미 그 계열의 유일 이름을 쥐고 있다 → 아무것도 바꾸지 않는다.
    KeepCurrent,
    /// 요청 이름이 남에게 있다 → 접미사를 붙인 이름으로 확정.
    Suffixed(String),
    /// 계열의 모든 번호(1..=u32::MAX)가 점유됐다 → 발급 불가.
    Exhausted,
}

/// 개명 결말 — 실패 사유를 호출부가 **구분**할 수 있어야 한다. bool 이면 "그런 에이전트가 없다" 와
/// "이름을 발급할 수 없다" 가 같은 값으로 뭉개져 wire 응답이 거짓 원인을 말한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameOutcome {
    /// 이름이 확정됐다(요청 이름 그대로이거나 접미사가 붙은 형태). 값 = 확정된 canonical 이름.
    Renamed(String),
    /// 이미 그 계열의 유일 이름을 쥐고 있어 아무것도 바꾸지 않았다. 값 = 유지된 이름.
    Unchanged(String),
    /// 그 id 의 에이전트가 명부에 없다.
    NotFound,
    /// 접미사 공간 소진 — 이름을 발급할 수 없어 개명을 거부했다(상태 미변경).
    Exhausted,
}

/// 계열에서 **쓸 번호**를 고른다. `used` = 그 계열이 지금 점유한 번호 집합(오름차순).
///
/// 기본 규칙은 ADR-0115 그대로 **관측 최대 + 1**(같은 명부 안에서 단조 증가 — 산 것끼리 번호가 겹치지
/// 않는다). 별도 최고수위 상태는 없으므로 계열이 비면 번호는 1부터 다시 나온다(ADR-0123).
///
/// ★포화 탈출구(saturation-only)★: 최대가 `u32::MAX` 면 "최대 + 1" 이 없다. 그때만 **가장 낮은 빈
/// 번호**로 내려간다 — 안 그러면 `이름(4294967295)` 하나가 그 계열의 42억 개 빈 번호를 영구히 봉쇄한다.
/// 정상 경로는 단조 증가를 유지하고, 이 갈래는 최대가 MAX 일 때만 열린다.
/// `None` = 1..=MAX 가 전부 점유(현실 도달 불가 — 전역 함수로 두기 위한 종결 갈래).
// ADR-0123
// ADR-0115
fn pick_suffix(used: &std::collections::BTreeSet<u32>) -> Option<u32> {
    if let Some(&max) = used.iter().next_back() {
        if let Some(next) = max.checked_add(1) {
            return Some(next);
        }
    } else {
        return Some(1);
    }
    // 포화 — 오름차순으로 걸으며 첫 구멍을 찾는다(`used` 는 명부 크기로 유계라 금방 끝난다).
    let mut candidate: u32 = 1;
    for &n in used.iter() {
        match n.cmp(&candidate) {
            std::cmp::Ordering::Less => continue,
            std::cmp::Ordering::Equal => candidate = candidate.checked_add(1)?,
            std::cmp::Ordering::Greater => break,
        }
    }
    Some(candidate)
}

pub struct AgentManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>,
    // C1: Tauri AppHandle이 아니라 StatusSink trait 주입(테스트 시 Noop 가능).
    status_sink: Arc<dyn StatusSink>,
    // S9: 프로필 단일 소유자(sid 생성·갱신·persist) + claude 세션 추적기.
    profiles: Arc<ProfileRegistry>,
    // ADR-0061: 프리셋(cwd 북마크) 단일 소유자. 프로필과 동일하게 데몬이 보유(유저 데이터 단일 소유,
    // ADR-0029)한다. reaper 는 프리셋을 안 보므로(에이전트 수명과 무관) manager 필드로만 둔다.
    presets: Arc<PresetRegistry>,
    tracker: Arc<SessionTracker>,

    // ── ADR-0019 reaper ──────────────────────────────────────
    /// 데몬/앱 셧다운 전역 플래그. shutdown_all 이 각 kill **전에** set 한다 → 그 사이 종료된
    /// 세션의 finish hook 이 true 를 snapshot 해 reaper 가 disposition 을 스킵(부팅 복원 유지).
    shutting_down: Arc<AtomicBool>,
    /// 세션/pump finish hook 이 ReapMsg 를 보내는 채널(단일 supervisor 가 소비).
    reaper_tx: Sender<ReaperCmd>,
    /// reaper 스레드 핸들. Drop 시 join(Stop 송신 후 대기) — 테스트 누수 방지.
    reaper_handle: Option<JoinHandle<()>>,

    /// ADR-0086 제어 채널 provisioning seam. spawn 시 provision(토큰+mcp-config 발급), terminal 시
    /// reaper 가 revoke(폐기+파일 삭제). 데몬만 실제 구현(`DaemonControlChannel`)을 주입하고, 기본은
    /// NoopControlChannel(제어 채널 없음 — headless 테스트·shell-only 경로). Arc 라 reaper 와 공유.
    control: Arc<dyn ControlChannel>,

    /// ADR-0086 provision 레이스 가드(FIX 6) — 현재 spawn 진행 중인 AgentId 예약 집합. contains_key
    /// 가드(read lock)와 실제 sessions.insert(write lock) 사이의 TOCTOU 창에서 **다른 연결**이 같은
    /// AgentId 를 동시에 spawn 하면, 둘 다 provision 을 불러 같은 (AgentId,epoch) config 경로에 쓰고
    /// 한쪽 reaper 가 상대 산 세션을 오삭제할 수 있다. 진입 시 이 집합에 원자적으로 예약(이미 있으면 즉시
    /// Err)해 두 번째 동시 spawn 을 깨끗이 거부한다. 예약은 성공(등록 완료)·실패(어느 조기 반환)든
    /// SpawnReservation(RAII)이 drop 시 해제한다. ★sessions 맵과 별개 leaf lock★: 이 Mutex 보유 중
    /// sessions/status 락을 잡지 않는다(ADR-0006 — 짧은 임계구역, 순수 HashSet 조작).
    spawning: Arc<Mutex<HashSet<AgentId>>>,

    /// 이름 배정 게이트 — 상태 없는 직렬화 락. 지키는 불변식은 하나: **명부를 관측한 뒤 그 결과로
    /// 이름을 커밋하기까지 다른 배정이 끼어들지 못한다**(없으면 동시 생성 둘이 같은 이름을 비었다고
    /// 보고 둘 다 가져간다). 결정표 전체 — 파생·관측·커밋 — 가 이 락 안에서 일어난다.
    ///
    /// ★락 순서 = name_allocation → sessions/profiles 단방향★. 이 락을 잡는 곳은 셋
    /// (`create_agent`·`rename_agent`·`register_for_spawn`)이고 셋 다 잡은 **뒤에야** 명부를 만진다.
    /// 역순(profiles 보유 중 이 락 취득)은 존재하지 않아 ADR-0006 순서에 순환이 없다.
    ///
    /// ★임계구역은 값싸지 않다★: ① `roster()` 관측이 **override 없는 잠든 에이전트 1건당
    /// `dunce::canonicalize` syscall 1회**를 치르고 ② 커밋이 `agents.json` 전체를 디스크에 쓴다(ADR-0071).
    /// 그래서 생성·개명·신규 등록 spawn 은 전역 직렬화되고, cwd 가 죽은 네트워크 공유에 있는 에이전트가
    /// 하나라도 있으면 그 syscall 이 멈춘 동안 세 경로가 함께 막힌다.
    /// ★그래도 메일 배달은 막히지 않는다 — 이 성질을 깨뜨리지 말 것★: `DeliveryPort`(주입·로스터·이름)는
    /// 이 락을 절대 잡지 않는다(`roster()` 는 락 없이 부를 수 있고 배달 경로가 그렇게 쓴다).
    // ADR-0120
    // ADR-0123
    // ADR-0006
    name_allocation: Arc<Mutex<()>>,
}

/// spawn 진행 중 AgentId 예약을 잡고, drop 시 자동 해제하는 RAII 가드(ADR-0086 FIX 6). spawn_agent
/// 의 어느 조기 반환(provision 실패·PTY 실패·`?`)에서도 예약이 새지 않게 한다. `reserve` 가 이미 예약된
/// id 면 None(두 번째 동시 spawn 거부).
struct SpawnReservation {
    spawning: Arc<Mutex<HashSet<AgentId>>>,
    id: AgentId,
}

impl SpawnReservation {
    /// (AgentId) 예약 시도. 이미 다른 spawn 이 예약 중이면 None. 성공 시 가드 반환(drop 에 해제).
    fn reserve(spawning: Arc<Mutex<HashSet<AgentId>>>, id: AgentId) -> Option<Self> {
        {
            let mut set = spawning.lock().expect("spawning set poisoned");
            if !set.insert(id) {
                return None; // 이미 진행 중 — 두 번째 동시 spawn 거부.
            }
        }
        Some(Self { spawning, id })
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
        // 예약 해제(성공·실패 무관). 없어도 무해(remove 는 없으면 false).
        let _ = self
            .spawning
            .lock()
            .expect("spawning set poisoned")
            .remove(&self.id);
    }
}

/// provision 성공 후 세션 등록 **전에** 실패(exe/PTY 오류·`?` 조기 반환)하면 발급된 토큰+config
/// 파일이 영원히 샌다(세션이 없어 reaper 가 영영 revoke 안 함) — 이를 막는 RAII 가드(ADR-0086 FIX 3).
/// provision 이 실제 endpoint 를 돌려줬을 때만 arm 되고, 세션 등록이 끝나면 `disarm()` 으로 무장 해제한다.
/// drop 시 아직 armed 면 revoke(폐기+파일 삭제)를 부른다 — 모든 pre-registration 실패 경로를 커버한다.
///
/// ★lock 미보유(ADR-0006)★: drop 은 sessions/status 락을 잡지 않는 지점(spawn_agent 조기 반환)에서만
///   일어나므로 revoke(registry leaf lock + 파일 IO)가 락 순서를 깨지 않는다.
struct ProvisionGuard {
    control: Arc<dyn ControlChannel>,
    id: AgentId,
    epoch: u32,
    /// true 인 동안 drop 하면 revoke. 세션 등록 성공 시 disarm() 이 false 로 내려 revoke 를 막는다
    /// (등록된 세션의 revoke 는 이제 kill_agent/reaper 소관 — 이중 revoke 방지, 정상 수명으로 이관).
    armed: bool,
}

impl ProvisionGuard {
    /// 세션 등록 완료 후 호출 — 무장 해제(정상 수명으로 이관). 이후 drop 은 revoke 하지 않는다.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProvisionGuard {
    fn drop(&mut self) {
        if self.armed {
            // 세션 등록 전 실패 — 새는 토큰+config 를 회수한다(revoke 는 idempotent).
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                "ADR-0086: spawn 실패(세션 등록 전) — 발급된 제어 채널 토큰/config 회수(revoke)"
            );
            self.control.revoke(self.id, self.epoch);
        }
    }
}

impl AgentManager {
    /// 기본 생성자 — 제어 채널 없음(NoopControlChannel). headless 테스트·제어 채널 미사용 경로.
    pub fn new(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
    ) -> Self {
        Self::new_with_control(
            status_sink,
            profiles,
            presets,
            tracker,
            Arc::new(NoopControlChannel),
        )
    }

    /// 제어 채널 주입형(ADR-0086) — 데몬이 `DaemonControlChannel` 을 끼운다. reaper 도 같은 Arc 를
    /// 공유해 terminal 수렴 지점에서 revoke 한다(spawn=provision / terminal=revoke 인과 대칭).
    pub fn new_with_control(
        status_sink: Arc<dyn StatusSink>,
        profiles: Arc<ProfileRegistry>,
        presets: Arc<PresetRegistry>,
        tracker: Arc<SessionTracker>,
        control: Arc<dyn ControlChannel>,
    ) -> Self {
        let sessions = Arc::new(RwLock::new(HashMap::new()));

        // reaper supervisor 1개 기동 — manager 와 동일한 sessions/profiles/status_sink 를 공유한다
        // (두 주체가 같은 모델을 본다). reap_one 이 lock 밖에서 disposition·통지를 수행한다.
        // ★control 도 공유(ADR-0086)★: reaper 가 terminal(단일 소비자) 시 revoke 를 부른다.
        let deps = ReaperDeps {
            sessions: sessions.clone(),
            profiles: profiles.clone(),
            status_sink: status_sink.clone(),
            control: control.clone(),
        };
        let (reaper_tx, reaper_handle) = reaper::spawn_reaper(deps);

        Self {
            sessions,
            status_sink,
            profiles,
            presets,
            tracker,
            shutting_down: Arc::new(AtomicBool::new(false)),
            reaper_tx,
            reaper_handle: Some(reaper_handle),
            control,
            spawning: Arc::new(Mutex::new(HashSet::new())),
            name_allocation: Arc::new(Mutex::new(())),
        }
    }

    /// 프리셋 레지스트리 접근(connection_core 의 프리셋 CRUD 에 사용, ADR-0061).
    pub fn presets(&self) -> &Arc<PresetRegistry> {
        &self.presets
    }

    // ── 명부(roster) — 단일 입구 ────────────────────────────────────────────
    //
    // ★프로필 레지스트리 핸들은 밖으로 나가지 않는다(ADR-0119 결정 1)★: 옛 `profiles()` 접근자는
    //   제거됐다. 바깥은 아래 좁은 동사들만 쓴다 — 프로필 *데이터*(AgentProfile)는 아직 wire 계약
    //   (`ProfileList`) 때문에 경계를 넘지만, **레지스트리 자체를 쥐고 임의 mutation 하는 경로**는 없다.

    /// "전체 에이전트 + 각자 살아있음/잠듦" 을 만드는 **유일한 곳**(ADR-0119 결정 2).
    ///
    /// ★스냅샷 1회★: 로스터와 "산 세션 id 집합"(잠듦 차집합의 기준)이 **같은 `list_agents()` 한 장**에서
    ///   나온다. 두 번 뜨면 그 사이 spawn·종료·삭제가 끼어 같은 발송의 두 수신자가 다른 세계를 본다
    ///   (ADR-0111 결정 2 금지 부류).
    /// ★잠듦 = **id 축** 차집합★: 프로필의 세션은 그 프로필 id 로 뜨므로(`activate_profile`) "산 세션이
    ///   없는 프로필" 을 id 로 정확히 가른다. 이름 축으로 빼면 **산 동명 하나가 잠든 다른 프로필을 통째로
    ///   가린다**(그 회귀는 `messaging_host` 테스트가 봉인).
    /// ★잠든 이름은 접지 않는다★: 같은 이름 잠듦 2건은 2건 그대로 올라온다(동명 판정 축이라 dedup 은
    ///   판정을 조용히 바꾼다).
    /// ★산/잠듦 이름 출처가 **다르다**(의도 — 합치지 말 것)★: 산 항목은 `list_agents()` 가 만든
    ///   `AgentInfo.name`(= `resolve_canonical_name`, session.cwd 기반), 잠든 항목은
    ///   `AgentProfile::canonical_name_when_live()`(profile.cwd + 같은 정규화). 두 파생을 하나로 합치면
    ///   파킹 키 동작이 바뀐다 — 산 세션은 `session.cwd`(spawn 시 canonicalize)를, 잠든 프로필은 raw
    ///   `profile.cwd` 를 정규화해 쓰는 서로 다른 재료를 본다.
    /// ★fs 접근은 override 없는 잠든 프로필에서만★: `canonical_name_when_live()` 의 단축(display_name 이
    ///   비공백이면 syscall 0)을 **여기서 무력화하지 말 것** — 이 조회는 발송 임계 경로에 있고, cwd 가 죽은
    ///   네트워크 공유면 canonicalize 한 번이 수십 초 블록이다. 그래서 그 함수를 재구현하지 않고 그대로 쓴다.
    /// ★락 규율(ADR-0006 · ADR-0071)★: `sessions`(RwLock)와 프로필 맵(Mutex)은 **독립 도메인**이다.
    ///   `list_agents()` 가 sessions 락을 잡아 Arc 를 clone 하고 즉시 놓은 뒤에야 `profiles.list()` 가
    ///   프로필 락을 잡는다 — **순차이고 중첩이 아니다**. 두 맵을 한 락으로 합치면 프로필 저장(락 보유 중
    ///   디스크 write)이 세션 조회(= 봉투 주입 경로)를 막는다(ADR-0119 거부 대안).
    /// ★원자적이 아니고, 원자적으로 만들지 않는다★: 두 조회 사이 경합 잔여는 ADR-0116 이 이미 판단해
    ///   TTL 24h 로 유계임을 수용했다. "이제 한 입구니 원자적으로" 는 0116 이 기각한 방향이다.
    // ADR-0119
    pub fn roster(&self) -> Vec<RosterEntry> {
        // ① 산 세션 스냅샷 1회(sessions 락은 list_agents 안에서 잡고 즉시 해제).
        let snapshot = self.list_agents();
        let mut live_ids: HashSet<AgentId> = HashSet::with_capacity(snapshot.len());
        let mut entries: Vec<RosterEntry> = Vec::with_capacity(snapshot.len());
        for info in snapshot {
            // 시체(terminal)는 reap 까지 맵에 남는다 — 존재가 아니라 상태로 가른다(ADR-0116 술어).
            if !info.status.is_live() {
                continue;
            }
            live_ids.insert(info.id);
            entries.push(RosterEntry {
                id: info.id,
                canonical_name: info.name.clone(),
                live: Some(info),
            });
        }
        // ② 그 다음에야 프로필 락(중첩 아님 — 위 스냅샷은 이미 확정됐다).
        for p in self.profiles.list() {
            if live_ids.contains(&p.id) {
                continue;
            }
            entries.push(RosterEntry {
                id: p.id,
                canonical_name: p.canonical_name_when_live(),
                live: None,
            });
        }
        entries
    }

    /// 에이전트 1건의 저장 스냅샷(없으면 None). spawn/활성화 인자 조립·삭제 전 이름 파생처럼
    /// **읽기만** 하는 호출부용 — 레지스트리 핸들을 넘기지 않기 위한 좁은 입구다(ADR-0119).
    pub fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        self.profiles.get(id)
    }

    /// 전체 저장 스냅샷(목록 응답용). wire `ProfileList` 가 아직 프로필 타입을 그대로 나르므로
    /// 데이터는 경계를 넘지만, 레지스트리 핸들은 넘지 않는다(어휘 개명은 후속 슬라이스).
    pub fn agent_snapshots(&self) -> Vec<AgentProfile> {
        self.profiles.list()
    }

    /// 에이전트의 현재 claude 세션 id(없으면 None). 진단 bin 이 트랜스크립트 탭 위치를 잡을 때 쓰는
    /// 좁은 읽기 — 이것 하나 때문에 레지스트리 전체를 열지 않는다.
    pub fn agent_claude_session_id(&self, id: AgentId) -> Option<uuid::Uuid> {
        self.profiles.get(id).and_then(|p| p.claude_session_id)
    }

    /// 에이전트 신규 등록(트리 "만들기"). 등록 전에 명부 전역 이름 유일성을 강제한다(ADR-0120).
    ///
    /// 반환 = **이 호출이 등록한 프로필**(배정된 이름이 반영된 값). 호출자 응답이 그 이름을 담아야 하므로
    /// 필요하다 — 접미사가 붙었는데 등록 전 스냅샷을 돌려주면 화면과 명부가 다른 이름을 갖는다.
    /// ★저장된 값을 되읽은 것은 아니다★: `ProfileRegistry::mutate` 가 저장 직전 `normalize_hierarchy` 를
    ///   돌리므로 원리적으로는 `parent_id` 가 갈릴 수 있다. 이 경로는 항상 `parent_id == None` 인 새
    ///   프로필이라 실제로 갈리지 않지만, 되읽기가 필요해지면 명시적으로 다시 조회할 것.
    ///
    /// ★접미사는 `display_name` 으로 박는다★: canonical 이름은 override 가 없으면 cwd basename 파생이라,
    ///   같은 폴더를 가리키는 둘은 개명 없이도 자동 동명이 된다(ADR-0120 §영향). 그 충돌을 해소할 수 있는
    ///   유일한 저장 자리가 override 다.
    /// ★Err = 접미사 공간 소진★ — 등록은 일어나지 않는다.
    // ADR-0120
    // ADR-0123
    pub fn create_agent(&self, mut profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        profile.display_name = normalize_display_name(profile.display_name.take());
        // ★파생도 게이트 안에서★: 요청 이름을 정하는 읽기가 게이트 밖에 있으면 관측과 커밋 사이가 아니라
        //   **파생과 관측 사이**에 창이 생긴다(그 사이 남이 같은 이름을 커밋하면 둘 다 자유로 판정한다).
        let _gate = self.lock_name_allocation();
        let desired = profile.canonical_name_when_live();
        // 신규 등록이라 "자기 현재 이름" 이 없다 → 결정표의 3·4b 갈래만 나온다.
        match self.decide_name(profile.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => profile.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            // `current: None` 을 넘겼으므로 결정표가 이 값을 낼 수 없다(4a 는 개명 전용).
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) 은 KeepCurrent 를 낼 수 없다")
            }
        }
        self.profiles.upsert(profile.clone());
        Ok(profile)
    }

    /// 에이전트 삭제(트리 "지우기"). 부모 삭제 시 자식은 루트로 승격한다(ProfileRegistry::remove).
    pub fn delete_agent(&self, id: AgentId) {
        self.profiles.remove(id);
    }

    /// 표시명 override set/clear(트리 "이름 변경").
    ///
    /// ★개명도 유일성 검사 지점이다(ADR-0120 결정 2)★: 0115 는 신규 등록만 봤으나 트리 개명이 실재해
    ///   그쪽으로 유일성이 뚫린다. `None`(override 해제)도 검사 대상이다 — 해제하면 canonical 이름이 cwd
    ///   basename 으로 **바뀌므로** 그것도 개명이다.
    ///
    /// ★★결정표(`decide_name`)가 판정을 전담한다 — 여기서 미리 단축하지 않는다★★. 예전엔 "자기가 이미
    ///   요청 계열 이름을 쥐고 있으면 no-op" 을 게이트 **앞에서** 먼저 걸렀는데, 그러면 **요청 이름이 비어
    ///   있어도** no-op 이 됐다: `bob(1)` 을 쥔 에이전트가 `bob` 이 비었는데도 개명되지 않고 성공만
    ///   보고했다(호출부는 Ack + 목록 broadcast 까지 해서 사용자·LLM 이 안 된 일을 됐다고 본다). override
    ///   해제도 같은 이유로 영구 불가였다(`C:/shared` 의 `shared(1)` 은 해제 결과가 제 계열이라 늘 걸렸다).
    ///   "비었으면 준다" 는 판정은 명부를 봐야 알 수 있으므로 게이트 안이어야 한다.
    // ADR-0120
    // ADR-0123
    // ADR-0116
    pub fn rename_agent(&self, id: AgentId, display_name: Option<String>) -> RenameOutcome {
        // 결정표가 보는 `desired` 와 커밋되는 값이 같은 정규화 문자열이어야 `" bob "` 재요청이 자기 현재
        //   이름과 같은 값으로 판정된다(번호를 태우거나 주소를 흔들지 않는다).
        let display_name = normalize_display_name(display_name);
        let _gate = self.lock_name_allocation();
        let Some(profile) = self.profiles.get(id) else {
            return RenameOutcome::NotFound;
        };
        // ★자기 현재 이름은 **명부가 말하는 그 값**이어야 한다★: 산 에이전트의 로스터 이름은 session.cwd
        //   기반이고 프로필 파생은 profile.cwd 기반이라 갈릴 수 있다(`roster()` doc). 따로 파생해 비교하면
        //   남이 쥔 이름을 자기 것으로 오인해 커밋할 수 있다. 프로필만 있는(잠든) 경우엔 둘이 같은 값이다.
        let roster = self.roster();
        let current = roster
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.canonical_name.clone())
            .unwrap_or_else(|| profile.canonical_name_when_live());
        // 요청이 만들어 낼 canonical 이름. 해제(None)면 cwd 파생값이 된다.
        //
        // ★★규칙을 복제하지 않고, **생사에 맞는 기존 파생을 고른다**★★. 두 축은 재료도 함수도 다르다:
        //   - 산 축(`resolve_canonical_name`) = `canonical_name_or_id_fallback(override, **raw** session.cwd)`
        //     — canonicalize 를 하지 않는다.
        //   - 잠든 축(`canonical_name_when_live`) = 먼저 canonicalize 한 뒤 basename.
        //   그래서 산 에이전트의 예측을 잠든 함수에 태우면 `basename(canonicalize(cwd)) == basename(cwd)` 라는
        //   **가정**에 기대게 된다. 그 가정은 보편이 아니다: spawn 의 canonicalize 가 실패하면 raw
        //   `profile.cwd` 가 그대로 session.cwd 가 되고(spawn_agent 의 cwd 처리), 하네스 seam 은 임의 cwd 를
        //   꽂을 수 있다. 갈리는 순간 **남이 쥔 이름을 Free 로 오판해** 커밋하고 엉뚱한 이름을 보고한다 —
        //   ADR-0120 전역 유일성이 깨지는 지점이다. 그래서 산 항목은 산 축 함수로 예측한다.
        // ★반대 방향(산 축에 canonicalize 추가)은 금지★: 그 경로는 메일 배달 임계 경로라 syscall 을 앞에
        //   놓을 수 없다(ADR-0119). 이 수정은 예측 쪽만 바꾸며, 해제 경로에서 syscall 을 **하나 줄인다**.
        // ★비공백 override 요청은 두 함수가 글자 그대로 같다★(둘 다 override 를 그대로 돌려주고 cwd 를 보지
        //   않는다) — 그 경로 동작은 불변이다. 공백-only override 도 양쪽에서 "override 없음" 으로 취급된다.
        let self_live_cwd = roster
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.live.as_ref())
            .map(|info| info.cwd.clone());
        let desired = match self_live_cwd {
            // 산 에이전트 — 명부가 쓰는 그 함수·그 재료(raw session.cwd)로 예측한다.
            Some(live_cwd) => crate::agent::name::canonical_name_or_id_fallback(
                display_name.as_deref(),
                &live_cwd,
                id,
            ),
            // 잠든 에이전트 — 프로필 축 파생(canonicalize 포함) 그대로.
            None => {
                let mut probe = profile;
                probe.display_name = display_name.clone();
                probe.canonical_name_when_live()
            }
        };
        match self.decide_name_with_roster(&roster, id, &desired, Some(&current)) {
            // 3) 아무도 안 갖고 있다 → 요청한 override 를 그대로 확정(접미사 없음).
            // ★커밋 결과를 삼키지 않는다★: `get`·`roster()`·`rename` 은 프로필 락을 **각각** 잡으므로 그
            //   사이 다른 연결의 `DeleteProfile` 이 끼면 커밋이 대상을 못 찾고 false 를 낸다. 그걸 성공으로
            //   보고하면 wire 가 없는 에이전트에 Ack + 목록 broadcast 를 낸다(게이트는 이름 배정끼리만
            //   직렬화한다 — 삭제는 이 게이트를 잡지 않는다).
            NameDecision::Free => {
                if self.profiles.rename(id, display_name) {
                    RenameOutcome::Renamed(desired)
                } else {
                    RenameOutcome::NotFound
                }
            }
            // 4a) 남이 갖고 있고 나는 이미 그 계열 이름을 쥐었다 → 멱등 no-op.
            NameDecision::KeepCurrent => RenameOutcome::Unchanged(current),
            // 4b) 남이 갖고 있고 나는 계열 밖이다 → 접미사 배정(커밋 결과는 위와 같은 이유로 그대로 전달).
            NameDecision::Suffixed(assigned) => {
                if self.profiles.rename(id, Some(assigned.clone())) {
                    RenameOutcome::Renamed(assigned)
                } else {
                    RenameOutcome::NotFound
                }
            }
            NameDecision::Exhausted => {
                tracing::warn!(
                    agent = %id,
                    base = %desired,
                    "접미사 공간 소진 — 개명 거부(이름 미변경)"
                );
                RenameOutcome::Exhausted
            }
        }
    }
    /// 트리 계층 이동(부모 지정/해제). 검증은 ProfileRegistry::reparent 가 한 임계구역에서 한다.
    pub fn reparent_agent(&self, child_id: AgentId, parent_id: Option<AgentId>) -> bool {
        self.profiles.reparent(child_id, parent_id)
    }

    /// 부팅 자동 복원 대상 토글. 존재하면 true, 없는 id 면 false.
    pub fn set_agent_auto_restore(&self, id: AgentId, auto_restore: bool) -> bool {
        self.profiles
            .update_with(id, |p| p.auto_restore = auto_restore)
    }

    /// ★하네스 전용 명부 주입 seam(ADR-0012 — `insert_test_session` 과 동형 게이트)★ — 이름 유일성
    ///   (ADR-0120)을 **우회해** 프로필을 그대로 심는다. 유일성이 정상 경로로는 만들 수 없게 만든 상태
    ///   (예: 동명 잠듦 2건)를 재현해야 하는 봉인 테스트 전용이다. 운영 빌드엔 컴파일되지 않는다
    ///   (feature OFF = 메서드 부재) — 운영 경로가 이걸 부르면 명부 유일성이 조용히 깨진다.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn seed_agent_bypassing_uniqueness(&self, profile: AgentProfile) {
        self.profiles.upsert(profile);
    }

    /// 배정 게이트 취득. ★poison 을 무시한다★: 이 Mutex 는 `()` 를 감싸 **보호하는 상태가 없다** —
    /// 다른 스레드의 패닉이 남길 수 있는 불일치 데이터가 애초에 없다. `expect` 로 두면 무관한 패닉 한 번이
    /// 생성·개명·스폰을 데몬 재시작까지 영구히 막는다(순수 downside).
    fn lock_name_allocation(&self) -> std::sync::MutexGuard<'_, ()> {
        self.name_allocation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// ★이름 결정표(ADR-0120 유일성 · ADR-0123 번호 규칙) — 게이트 보유 중에만 부른다★.
    ///
    /// `self_id` 는 관측에서 **제외**한다(자기 이름과 충돌하지 않게). `current` = 명부가 지금 이 에이전트에
    /// 대해 말하는 canonical 이름(신규 등록이면 `None`).
    ///
    /// 판정 순서가 곧 계약이다:
    ///   3) `desired` 를 **아무도** 안 갖고 있으면 → `Free`. 자기가 그 계열에 있든 없든 상관없다
    ///      (여기서 계열을 먼저 보면 "요청 이름이 비었는데 안 바꿔 주는" 조용한 성공이 된다).
    ///   4a) 남이 갖고 있고 `current` 가 `desired` 계열(`desired` 또는 `desired(n)`)이면 → `KeepCurrent`.
    ///       재요청이 번호를 태우거나 빈 낮은 번호로 끌어내리는 것을 막는다(이름 = 메일 주소, ADR-0116).
    ///   4b) 그 밖 → 계열의 빈 번호로 `Suffixed`.
    // ADR-0120
    // ADR-0123
    fn decide_name(&self, self_id: AgentId, desired: &str, current: Option<&str>) -> NameDecision {
        self.decide_name_with_roster(&self.roster(), self_id, desired, current)
    }

    /// `decide_name` 의 본체 — 이미 뜬 로스터 스냅샷 위에서 판정한다(개명은 자기 현재 이름을 그 **같은**
    /// 스냅샷에서 읽어야 하므로 두 번 뜨지 않는다).
    fn decide_name_with_roster(
        &self,
        roster: &[RosterEntry],
        self_id: AgentId,
        desired: &str,
        current: Option<&str>,
    ) -> NameDecision {
        // ★두 축을 분리한다★: `taken_exact`(그 문자열을 남이 쓰는가 — 접미사를 붙일지 결정)와 계열 번호
        //   집합(몇 번을 붙일지 결정). 섞으면 두 방향으로 틀린다: `bob(1)` 만 있어도 `bob` 이 찬 것으로
        //   보이거나, 리터럴 `bob(0)` 이 접미사 없는 `bob` 을 점유한 것처럼 보인다.
        let mut taken_exact = false;
        let mut used: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for entry in roster {
            if entry.id == self_id {
                continue;
            }
            match classify_name(desired, &entry.canonical_name) {
                Some(NameKind::Exact) => taken_exact = true,
                Some(NameKind::Suffixed(n)) => {
                    used.insert(n);
                }
                None => {}
            }
        }
        if !taken_exact {
            return NameDecision::Free;
        }
        if let Some(current) = current {
            if classify_name(desired, current).is_some() {
                return NameDecision::KeepCurrent;
            }
        }
        match pick_suffix(&used) {
            Some(n) => NameDecision::Suffixed(format!("{desired}({n})")),
            None => NameDecision::Exhausted,
        }
    }

    /// spawn 이 프로필을 명부에 등록하는 지점(`spawn_agent` 단독 호출).
    ///
    /// ★★유일성은 **신규 등록에만** 건다 — epoch 교체는 개명 대상이 아니다(ADR-0115 §영향 · ADR-0007)★★.
    ///   restart / restore(`restore_all`·`restore_one`) / 재활성화(`activate_profile`) /
    ///   `resume_no_fallback` 는 전부 **같은 AgentId 의 맵 교체**지 새 에이전트가 아니다. id 가 이미 명부에
    ///   있는지로 그 둘을 가른다. ad-hoc spawn(연결이 그 자리에서 만든 프로필)은 명부에 없는 id 라 신규
    ///   등록으로 잡힌다.
    /// ★`upsert_preserving_hierarchy` 는 live 엔트리의 `display_name` 을 보존한다★ — 그래서 신규 등록
    ///   갈래에서만 override 를 심어도 안전하다(기존 id 갈래는 어차피 live 값이 이긴다). 이름이 실제로
    ///   새려면 이 분기 제거와 그 보존 규칙 상실이 **함께** 일어나야 한다(그 복합 회귀를
    ///   `epoch_replacement_never_renames_an_existing_agent` 가 잡는다).
    /// ★분기를 두는 또 하나의 이유 = 복원 경로 비용★: 부팅 복원은 에이전트마다 이걸 부르는데, 검사가 돌면
    ///   매번 명부 전체 스캔 + override 없는 잠든 에이전트 수만큼 canonicalize syscall 을 배정 게이트
    ///   안에서 치른다(게이트 필드 주석).
    /// ★Err = 접미사 공간 소진★ — spawn 이 그 Err 로 중단된다(중복 이름으로 뜨지 않는다).
    // ADR-0120
    // ADR-0123
    // ADR-0115
    fn register_for_spawn(&self, profile: &AgentProfile) -> Result<(), PtyError> {
        // 존재 확인은 이 id 만의 함수다. 기존 id 면 이름 배정 자체가 없으므로 게이트를 잡지 않는다.
        if self.profiles.get(profile.id).is_some() {
            // 이미 명부에 있는 에이전트 = epoch 교체. 이름 검사 없음(위 doc).
            self.profiles.upsert_preserving_hierarchy(profile.clone());
            return Ok(());
        }
        let mut fresh = profile.clone();
        fresh.display_name = normalize_display_name(fresh.display_name.take());
        // 파생·관측·커밋을 한 임계구역으로 묶는다(게이트 필드 주석의 불변식).
        let _gate = self.lock_name_allocation();
        let desired = fresh.canonical_name_when_live();
        match self.decide_name(fresh.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => fresh.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            // `current: None` 을 넘겼으므로 결정표가 이 값을 낼 수 없다(4a 는 개명 전용).
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) 은 KeepCurrent 를 낼 수 없다")
            }
        }
        self.profiles.upsert_preserving_hierarchy(fresh);
        Ok(())
    }

    // ── spawn ──────────────────────────────────────────────────────────────

    /// 프로필 기반 spawn. backend가 CommandSpec을 산출(claude면 mode에 따라
    /// `--session-id`/`--resume`). 성공 시 AgentInfo 반환.
    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // 이중 spawn 가드 — 같은 id가 이미 살아있으면 거부(맵 교체는 복원/재시작 경로 전용).
        // ★ADR-0082★: 이 Err 는 순수 방어선일 뿐 파괴 트리거가 아니다. activate_profile 이 진입 시
        //   contains_key 를 **선제로** 검사해 산 에이전트면 여기 닿기 전에 무해한 재활성화로 처리한다
        //   (옛날엔 이 Err 가 resume_with_fresh_fallback 에 의해 "resume 실패"로 오인돼 산 에이전트를
        //   kill 하는 a4aac1a 회귀를 낳았다 — 이제 그 오인 경로 자체가 없다).
        // ★잔여 레이스(ADR-0082 미해결·후속)★: 이 가드는 여기서 read lock 을 잡아 contains_key 를 본 뒤
        //   놓고, 실제 등록(sessions.insert)은 아래에서 별개 write lock 으로 한다 — 그 사이 창이 있다.
        //   같은 id 를 **서로 다른 연결**이 동시에 SpawnProfile 하면 둘 다 이 검사와 activate_profile 의
        //   pre-check 를 통과해 double-spawn 이 날 수 있다(데몬 명령 처리는 연결당 직렬일 뿐 연결 간엔
        //   아니다 — 각 연결이 제 read_task 에서 dispatch 를 await 한다). 이 window 는 ADR-0082 이전부터
        //   있던 **선재(pre-existing) 레이스**이며 이번 변경이 도입하지도 닫지도 않았다(후속 과제로 flag).
        if self
            .sessions
            .read()
            .expect("sessions poisoned")
            .contains_key(&profile.id)
        {
            return Err(PtyError::SpawnFailed(format!(
                "agent {} already running",
                profile.id
            )));
        }

        // ★provision 레이스 가드(FIX 6)★: 위 contains_key(read lock)와 아래 sessions.insert(write lock)
        //   사이의 TOCTOU 창에서 **다른 연결**이 같은 AgentId 를 동시에 spawn 하면, 둘 다 provision 을
        //   불러 같은 (AgentId,epoch) config 경로에 쓰고 한쪽 reaper 가 상대 산 세션을 오삭제할 수 있다.
        //   진입 즉시 (AgentId) 를 원자적으로 예약해 두 번째 동시 spawn 을 깨끗이 거부한다. 예약은 아래
        //   어느 조기 반환(provision 실패·PTY 실패)에서도 SpawnReservation drop 이 해제한다(RAII).
        //   ★leaf lock(ADR-0006)★: spawning Mutex 는 짧게 잡고 sessions/status 락과 겹치지 않는다.
        let _reservation = SpawnReservation::reserve(self.spawning.clone(), profile.id)
            .ok_or_else(|| {
                PtyError::SpawnFailed(format!(
                    "agent {} spawn already in progress (concurrent spawn rejected)",
                    profile.id
                ))
            })?;

        // 프로필을 레지스트리에 등록(idempotent + 즉시 persist). 복원 경로는 기존 프로필을 그대로 넘긴다.
        // ★hierarchy-preserving★: profile 은 SpawnProfile 등에서 뜬 **스냅샷**이라, spawn 사이 다른 연결이
        // reparent/rename 한 최신 parent_id/display_name 을 덮어쓰면 안 된다(lost update). 그 두 트리 메타는
        // live 엔트리 값을 보존하고 나머지(cwd/command/env/session)만 반영한다(ADR-0070/0072).
        // ★신규 등록이면 여기서 이름 유일성을 강제한다(ADR-0120) — epoch 교체는 건드리지 않는다★.
        //   접미사 공간 소진이면 Err 로 중단한다(중복 이름 에이전트를 띄우지 않는다).
        self.register_for_spawn(profile)?;

        // cwd 정규화 — claude 세션 디렉토리 표기 고정(UNC 회피). 실패 시 원본 사용(best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

        // backend가 세션 추적 대상인지 판단(claude=true, shell=false). true면 세션 id 확보.
        // 생성 책임은 ProfileRegistry(H-1.4).
        //
        // ★mode 별 sid 발급 규칙(ADR-0076 — "activate=resume, fresh=new sid" 봉인)★:
        //   - Resume: 저장된 sid 를 그대로 써야 기존 대화를 이어받는다 → ensure_session_id(있으면 그대로,
        //     드물게 없으면 최초 발급). backend 가 `--resume <sid>` 로 무손실 복원(ADR-0008).
        //   - Fresh: **반드시 새 sid**. ensure_session_id 를 쓰면 저장된 sid 를 재사용해
        //     `--session-id <저장 sid>` 로 떠 디스크 세션과 충돌한다("Session ID already in use" → claude
        //     즉사, 이 세션의 재현 버그). new_session_id 가 항상 새 uuid 를 발급(옛 sid 는 이력 보존).
        //   spawn_agent 이 이 판정의 단일 권위점이라 어떤 호출자(Spawn/SpawnProfile/restore/fallback)든
        //   mode 만 맞게 넘기면 sid 충돌이 원천 봉인된다(FIX 2 backend-authoritative).
        let needs = backend::needs_session(&profile.command);
        let sid = if needs {
            match mode {
                SpawnMode::Resume => self.profiles.ensure_session_id(profile.id),
                SpawnMode::Fresh => self.profiles.new_session_id(profile.id),
            }
        } else {
            None
        };

        // epoch는 레지스트리의 현재값(fallback respawn 등에서 미리 bump됨).
        let epoch = self.profiles.get(profile.id).map(|p| p.epoch).unwrap_or(0);

        // ADR-0086: 제어 채널 provisioning. 데몬이 (AgentId,epoch)용 토큰+mcp-config 를 발급해
        //   ControlEndpoint 를 돌려준다. ★spec 조립 직전에 부른다★ — build_command_spec 이 endpoint 를
        //   받아 backend 방식(claude=`--mcp-config`, ADR-0004)으로 명령줄에 주입해야 하므로. epoch 는
        //   위에서 확정된 현재값이라 재활성화(bump) 때마다 새 토큰이 발급된다(토큰 수명=(AgentId,epoch)).
        //
        // ★backend-conditional(round-2 F3)★: 제어 채널을 **소비하는** backend(claude)에만 provision 을
        //   부른다 — shell 은 supports_control_channel=false 라 provision 을 아예 건드리지 않는다(registry
        //   미접촉). 이렇게 하면 config-write 실패가 MCP 가 필요 없던 셸 스폰을 중단시키는 회귀가 없다.
        //   판정은 backend dispatch(ADR-0004) — manager 가 command 를 직접 matches! 하지 않는다.
        // ★fail-closed(FIX 2)★: provision 을 **부르는** backend 에서 provision 3-값(Ok(Some)/Ok(None)/
        //   Err) 중 Err(CSPRNG/파일 write 실패)면 **스폰을 중단**한다(제어 채널 없이 몰래 도는 에이전트
        //   금지 — health 위장 방지). Ok(None)=제어 채널을 안 쓰는 정당한 부재(Noop)라 그대로 진행.
        //   Ok(Some)=발급 성공 → 아래 ProvisionGuard 로 arm 해, 세션 등록 전 어느 실패에서든 발급된
        //   토큰/config 를 회수한다(FIX 3 leak 방지). supports_control_channel=false 인 backend 는 provision
        //   을 건너뛰므로 None(부재)과 동일하게 흐른다 — 그 backend 엔 fail-closed 계약이 적용되지 않는다.
        // ADR-0086
        let control_endpoint = if backend::supports_control_channel(&profile.command) {
            // ADR-0099: backend 의 MCP-capability 를 provision 에 넘겨 채널 물리 배선·프라이밍 변형·grant 를
            //   한꺼번에 가르게 한다(정합 불변식 = 깐 채널 == 프라이밍이 가르치는 채널). 판정은 backend
            //   dispatch(ADR-0004) — manager 는 command 를 직접 matches! 하지 않는다.
            // ADR-0099
            let accepts_mcp = backend::accepts_mcp_config(&profile.command);
            self.control
                .provision(profile.id, epoch, accepts_mcp)
                .map_err(|e| {
                    PtyError::SpawnFailed(format!(
                        "control channel provision failed (fail-closed): {e}"
                    ))
                })?
        } else {
            // 제어 채널 미소비 backend(shell): provision 미호출 → registry 미접촉 → endpoint 없음.
            None
        };
        // provision 이 실제 endpoint 를 줬으면 회수 가드를 arm(세션 등록 성공 시 disarm). None(부재)이면
        //   회수할 게 없어 arm 하지 않는다.
        let mut provision_guard = control_endpoint.as_ref().map(|_| ProvisionGuard {
            control: self.control.clone(),
            id: profile.id,
            epoch,
            armed: true,
        });

        // backend가 program/args/env/cwd를 중립 CommandSpec으로 산출. transport는 claude/shell을 모른다.
        // control_endpoint(추상 descriptor)를 함께 넘긴다 — backend 가 자기 프로그램 방식으로 주입한다.
        let spec = backend::build_command_spec(
            &profile.command,
            mode,
            sid,
            cwd.clone(),
            profile.env.clone(),
            control_endpoint,
        );

        // backend(프로그램)가 결정하는 caps(session/model)를 spec과 별도로 산출해 흘린다.
        // spec은 backend-neutral(program/args뿐)이라 caps를 spec에 싣지 않고 따로 전달한다 —
        // session이 transport caps와 compose 한다(claude=resume true, shell=resume false 정확화).
        let bcaps = backend::backend_caps(&profile.command);

        // ADR-0044 조립 분기(양 끝 3지점): json 모드면 StdioTransport 선택 + 입력을 claude 유저 JSON
        // 라인으로 감싸는 encoder. 그 외는 PtyTransport + Raw(터미널 경로 바이트 불변). 판정은
        // 프로필 command 단일 출처(is_json_mode/input_encoder) — spawn_session은 backend를 모른다.
        let json_mode = profile.command.is_json_mode();
        let encoder = backend::input_encoder(&profile.command);
        // 출력 정제기(입력 encoder 의 대칭 짝) — json 모드면 backend 가 claude decoder 를 만들고,
        // 그 외엔 None(바이트 직통). claude 스키마 지식은 backend 단독이라 여기선 command 만 넘긴다.
        let decoder = backend::output_decoder(&profile.command);

        // ADR-0079: resume(=과거 대화 이어받기) 스폰이면 `.jsonl` transcript 에서 과거 이벤트를 읽어
        //   버퍼에 seed 한다(pump 전). Fresh 는 이어받을 대화가 없으므로 빈 Vec(기존 동작 불변). json
        //   모드 claude 만 실제로 읽고(터미널은 TUI PTY repaint 로 복원, shell 은 대화 없음), 그 외엔
        //   backend dispatch 가 빈 Vec 을 돌려준다. transcript 경로·파싱 지식은 backend 단독(ADR-0004).
        let seed_events = match mode {
            SpawnMode::Resume => match sid {
                Some(s) => backend::resume_transcript_events(&profile.command, &cwd, s),
                None => Vec::new(),
            },
            SpawnMode::Fresh => Vec::new(),
        };

        let (session, child_pid) = self.spawn_session(
            profile.id,
            spec,
            bcaps,
            encoder,
            decoder,
            json_mode,
            epoch,
            seed_events,
        )?;

        // ★provision 가드 무장 해제(FIX 3)★: 여기 도달 = spawn_session 이 sessions 맵에 세션을 등록 완료.
        //   이제 이 토큰/config 의 수명은 세션에 붙어(kill_agent 선제 revoke + reaper terminal revoke 가
        //   책임진다) — 가드가 이중 revoke 하지 않게 무장 해제한다. 이 줄 위의 어느 `?` 조기 반환이든
        //   가드가 armed 인 채 drop 돼 revoke 가 발급 자원을 회수한다.
        if let Some(g) = provision_guard.as_mut() {
            g.disarm();
        }

        // claude 세션 추적 부착(best-effort). shell은 세션 파일이 없으니 생략(needs_session=false).
        if let (Some(s), Some(pid)) = (sid, child_pid) {
            if needs {
                self.tracker.watch(profile.id, pid, s);
            }
        }

        tracing::info!(agent = %profile.id, epoch, ?mode, "에이전트 spawn");

        let info = self.agent_info(&session);
        self.status_sink.agent_list_updated(self.list_agents());
        Ok(info)
    }

    /// ★수동 활성화 진입점 — 이어받기(resume) 전용, fresh-fallback 폐지(ADR-0082)★.
    /// SpawnProfile 핸들러가 `spawn_agent` 대신 이걸 부른다. 세 갈래로 나뉜다:
    ///
    /// 1. **이미 실행 중(재활성화 가드)** — 같은 id 세션이 살아 있으면 **아무것도 죽이거나
    ///    재spawn 하지 않고** 그 세션의 AgentInfo 를 그대로 돌려준다(무해한 "이미 실행 중" 신호,
    ///    epoch 불변). ★이게 a4aac1a 회귀의 핵심 수정★: 예전엔 이 경로가 `spawn_agent` 이중-spawn
    ///    가드의 "already running" Err 를 만나 `resume_with_fresh_fallback` 이 그걸 "resume 실패"로
    ///    오인 → `fallback_fresh` 가 **멀쩡히 돌던 산 에이전트를 kill** → epoch++ → 빈 fresh 로 교체
    ///    (유저 실측 회귀). 이제 가드 Err 에 닿기 전에 선제 contains_key 로 걸러 산 에이전트를 놔둔다.
    ///    (이 pre-check 는 흔한 경로 — **같은 연결**에서 직렬로 들어오는 재활성화 — 를 닫는다. spawn_agent
    ///    의 이중-spawn 가드는 최후 방어선으로 남지만, pre-check 와 실제 spawn 사이의 TOCTOU 를 완전히
    ///    닫지는 못한다: **다른 연결**이 같은 id 를 동시에 활성화하면 둘 다 pre-check 와 contains_key 를
    ///    통과해 double-spawn 이 날 수 있다(데몬 명령 처리는 연결당 직렬일 뿐 연결 간엔 아님). 이 레이스는
    ///    ADR-0082 이전부터 있던 선재(pre-existing) window 로, 이번 변경이 닫지 않는다 — 후속 과제.)
    /// 2. **Fresh(진짜 신규 — 세션 없음)** — `spawn_agent(Fresh)` 위임(이어받을 대화 없음, 기존 동작
    ///    보존). 이건 실패-fallback 이 아니라 정상 신규 생성이다(ADR-0076 "Fresh=새 sid" 유효).
    /// 3. **Resume** — `resume_no_fallback` 로 이어받기만 시도한다. 이어받을 수 없으면(빈/미대화/손상 —
    ///    claude 가 "No conversation found ..." 로 즉사) **새 대화를 만들지 않고** Failed(시체)로
    ///    남기고 사유를 로그로 남긴다(ADR-0082 — 원인은 LLM 이 읽어 에스컬레이션). 여기선 Err 로 노출.
    ///
    /// ★blocking★: Resume 모드는 EARLY_EXIT_WINDOW(현 3s)만큼 조기종료를 폴링하므로 호출이 그만큼
    ///   블록될 수 있다(restore_all 과 동일 성질). 데몬의 명령 처리 스레드에서 호출되므로 그 연결의
    ///   응답만 지연되고 다른 세션에는 영향 없다. Fresh 모드·재활성화 가드는 폴링 없이 즉시 반환한다.
    // ADR-0082
    // ADR-0076
    pub fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // 1. ★재활성화 가드(ADR-0082) — 산 에이전트를 절대 건드리지 않는다★. 같은 id 세션이 이미
        //    살아 있으면 kill/재spawn/epoch-bump 없이 현재 세션의 AgentInfo 를 무해하게 돌려준다.
        //    이중-spawn 가드 Err 가 파괴 트리거(옛 fresh-fallback)로 번역되던 회귀를 원천 차단한다.
        //    (read lock 은 clone 후 즉시 해제 — §10 락 순서 준수, agent_info 는 lock 미보유로 호출.)
        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: 이미 실행 중 — 재활성화 무시(산 에이전트 보존, ADR-0082)"
            );
            return Ok(self.agent_info(&session));
        }

        // 2. Fresh(진짜 신규 — 세션 없음)는 이어받을 대화가 없으므로 spawn_agent 위임(정상 신규 생성).
        if mode == SpawnMode::Fresh {
            return self.spawn_agent(profile, SpawnMode::Fresh);
        }

        // 3. Resume: 이어받기만 시도(fresh-fallback 폐지). resume_no_fallback 이 RestoreOutcome 을
        //    돌려주므로 결말을 AgentInfo/Err 로 번역한다.
        //
        // ★재활성화 = epoch++★: 여기 도달했다는 건 위 가드에서 산 세션이 **없음**을 이미 확인했다는
        //   뜻이다 — 즉 reap 으로 세션이 맵에서 빠진 **시체**를 같은 AgentId 로 다시 띄우는 맵 교체다.
        //   ADR-0007 불변식("같은 AgentId 맵 교체마다 epoch +1")을 그대로 적용해, 새 세션이 죽은
        //   세션과 다른 `[agentId, epoch]` 를 갖게 한다 → 프론트 구독(deps [viewId,agentId,epoch])이
        //   재발화해 resume 출력이 화면에 붙고, 옛 seq/cursor 가 새 스트림에 오적용되지 않는다.
        //   spawn_agent(L223)이 이 bump **뒤** 프로필 epoch 를 읽으므로 순서가 load-bearing 이다.
        //   (산 세션 재활성화는 위 가드에서 이미 걸러졌으므로 절대 여기 오지 않는다 — bump 안전.)
        //   또 이 bump 는 stale reap 의 apply_disposition epoch-guard(reaper.rs)가 재활성화된 산
        //   세션을 강등 못 하게 하는 구분자이기도 하다.
        // ADR-0084
        // ADR-0007
        self.profiles.bump_epoch(profile.id);

        match self.resume_no_fallback(profile) {
            // resume 성공 — 살아있는 세션의 info 반환.
            RestoreOutcome::Resumed => self.agent_info_by_id(profile.id),
            // resume 실패/조기종료 → 종점 Failed(시체). 새 대화 안 만듦. 호출자(핸들러)엔 Err 로 노출.
            RestoreOutcome::Failed { reason } => Err(PtyError::SpawnFailed(reason)),
            // resumable 프로필로만 진입하므로 Started/Blocked/FreshFallback 은 도달 불가(방어적 Err).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: 예상 밖 결말 {other:?}"
            ))),
        }
    }

    /// PtyTransport open + OutputCore 생성 + pump 기동(transport.start) + AgentSession 합성 +
    /// sessions 등록의 공통 기계부. 반환: 등록된 세션 Arc + child PID(Option).
    #[allow(clippy::too_many_arguments)]
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        decoder: Option<Box<dyn OutputDecoder>>,
        json_mode: bool,
        epoch: u32,
        // ADR-0079: resume 시 `.jsonl` 에서 복원한 과거 이벤트. pump 전에 core 버퍼에 seed 한다.
        //   Fresh(및 비-json)는 빈 Vec → seed 안 함(기존 fresh 버퍼 동작 불변).
        seed_events: Vec<OutputEvent>,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        // 1. 모드에 맞는 transport 조립(json=StdioTransport 파이프 / 그 외=PtyTransport ConPTY).
        //    child spawn + job 편입 + 파이프/reader·writer 확보. pump는 아직 안 띄움(start에서).
        //    json 모드면 출력 정제 decoder 도 함께 통로에 주입한다(ADR-0004 — 통로는 claude 모름).
        let (transport, child_pid) =
            select_transport(json_mode, &spec, DEFAULT_COLS, DEFAULT_ROWS, decoder)?;

        // 2. 출력 측 core 생성(status Running, seq 0). transport와 분리된 출력 fanout 담당.
        let core = Arc::new(OutputCore::new(id, epoch, self.status_sink.clone()));

        // 2.1. ★ADR-0079 seed-before-publish(load-bearing 순서 — cross-family review 2026-07-13)★:
        //      resume 복원 과거 이벤트를 **세션이 관측 가능해지기 전에**(= sessions 맵 insert 전) core
        //      Ring 에 seed 한다. 지금 core 는 이 함수 로컬 Arc 뿐이라 다른 스레드가 닿을 수 없다(구독·emit
        //      경로 모두 sessions 맵 조회를 거친다). 그래서 seed 를 여기서 끝내면 다음 두 윈도가 원천 차단된다:
        //        (a) empty-ring replay: insert 후 seed 전에 재접속 구독이 끼면 빈 Ring 을 replay 하고
        //            seed 는 fanout 안 하므로 과거를 영구 유실 → insert 전 seed 로 제거.
        //        (b) seq interleave: 그 윈도의 동시 emit/write 가 seed 와 seq 를 뒤섞어 Ring 순서를
        //            [0,2,1] 로 깨 replay 의 partition_point 전제를 위반 → seed 선행으로 제거.
        //      seed 는 여전히 start_pump 전이다(라이브 emit 은 pump 가 켜야 시작). seed_events 가 비면
        //      (Fresh·비-json·transcript 부재) no-op → 기존 fresh 버퍼 동작 불변.
        if !seed_events.is_empty() {
            tracing::info!(
                agent = %id,
                epoch,
                count = seed_events.len(),
                "ADR-0079: resume transcript seed (before publish)"
            );
            core.seed(seed_events);
        }

        // 2.5. ★ADR-0019 finish-snapshot hook 배선★. 세션별 intent atomic 신규 생성 + 전역
        //      shutting_down·reaper_tx 를 클로저로 캡처해 core 에 주입한다. core.finish 의 finalize
        //      승자 경로에서 1회 호출되며, **그 순간** intent·shutting_down 을 snapshot 해 ReapMsg 를
        //      송신한다(reap 시점 live read 금지 — 크래시→유저kill 오분류 race 방지).
        //      transport 는 이 의미를 모른다(그냥 core.finish 호출). send 실패(reaper 종료)는 무시.
        let intent = Arc::new(AtomicU8::new(TerminationIntent::None as u8));
        {
            let intent_hook = intent.clone();
            let shutting_down_hook = self.shutting_down.clone();
            let reaper_tx = self.reaper_tx.clone();
            core.set_on_terminal(Box::new(move |reason: TerminalReason| {
                let msg = ReapMsg {
                    id,
                    epoch,
                    reason,
                    // ★snapshot★: 이 두 load 가 finish 승자 순간의 frozen 값이다.
                    intent_at_finish: TerminationIntent::from_u8(
                        intent_hook.load(Ordering::SeqCst),
                    ),
                    shutting_down_at_finish: shutting_down_hook.load(Ordering::SeqCst),
                };
                let _ = reaper_tx.send(ReaperCmd::Reap(msg));
            }));
        }

        // 3. transport는 select_transport가 이미 Box<dyn AgentTransport>로 박싱해 반환한다.

        // 4. core + transport를 AgentSession으로 합성(cols/rows atomic은 session 보유).
        //    encoder(입력 인코딩 태그)도 함께 주입 — write_input이 transport로 넘기기 전 적용.
        let session = Arc::new(AgentSession::new(
            id,
            spec.cwd.clone(),
            epoch,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            intent,
            backend_caps,
            encoder,
            core,
            transport,
        ));

        // 5. ★ADR-0019 순서 변경★ sessions 등록을 pump 기동(start)보다 **먼저** 한다.
        //    (구 S9: start 후 insert.) 이유: finish hook 이 ReapMsg 를 보내는데, pump 가 즉시
        //    EOF→finish 하면 그 시점에 세션이 맵에 있어야 reaper 가 reap 한다. insert 전에 start 하면
        //    빠른 종료 시 hook send 가 맵에 없는 id 를 가리켜 reap 가 no-op→세션 좀비화. attach_pump 는
        //    start 내부 동기 완료라 join_pump 영향 없음(insert 순서 무관). write lock 즉시 해제.
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session.clone());

        // 5.5. ★ADR-0019 활성화 — 반드시 start_pump 전★: spawn(=지금 떠 있어야 함)이면 프로필을
        //      auto_restore=true 로 확정·persist 한다(강제종료 후 부팅 복원 대상이 되게). 이 플립을
        //      pump 기동 **전**에 둬야 race 가 닫힌다: 즉시 크래시(`cmd /c exit 1`)는 start_pump 직후
        //      pump 가 EOF→finish→reaper 가 auto_restore=false 로 내리는데, 이 플립이 그보다 늦으면
        //      false 를 true 로 덮어써 크래시 세션이 부팅 복원 대상으로 잘못 남는다(크래시 루프).
        //      순서를 "플립 true → start_pump → (크래시 시) reaper false" 로 고정해 reaper 의
        //      downgrade(false)가 항상 **마지막**이 되게 한다. spawn 은 활성화 행동이므로 여기서만 올린다
        //      (reaper 는 downgrade-only — true 로 올리지 않음).
        self.profiles.update_with(id, |p| p.auto_restore = true);

        // 6. pump 기동 — reader take + pump 스레드 spawn + core.attach_pump(핸들/done_rx 적재).
        //    이제부터 출력·종료가 흐른다. 종료 시 finish hook→ReapMsg(맵에 이미 존재).
        session.start_pump();

        Ok((session, child_pid))
    }

    // ── 복원 (S9 코어) ───────────────────────────────────────────────────────

    /// auto_restore 프로필 전부 복원 시도. **백그라운드 스레드에서 호출할 것**(stagger·조기종료
    /// 윈도 대기로 블로킹 — setup 동기 호출 금지, H-1.8). 에이전트별 결과를 통지하고 반환한다.
    pub fn restore_all(&self) -> Vec<RestoreReport> {
        let targets = self.profiles.restorable();
        tracing::info!(count = targets.len(), "restore_all 시작");

        let mut reports = Vec::with_capacity(targets.len());
        for profile in targets {
            let outcome = self.restore_one(&profile);
            // fallback에서 epoch가 bump됐을 수 있으니 최신값을 읽는다.
            let epoch = self
                .profiles
                .get(profile.id)
                .map(|p| p.epoch)
                .unwrap_or(profile.epoch);
            let report = RestoreReport {
                agent_id: profile.id,
                epoch,
                outcome,
            };
            tracing::info!(agent = %report.agent_id, ?report.outcome, "복원 결과");
            self.status_sink.restore_result(report.clone());
            reports.push(report);
            std::thread::sleep(RESTORE_STAGGER);
        }
        reports
    }

    /// 프로필 1개 복원. claude+sid 있으면 resume 시도(실패 시 Failed 시체, fresh-fallback 폐지),
    /// 그 외(shell 등)는 fresh로 시작.
    fn restore_one(&self, profile: &AgentProfile) -> RestoreOutcome {
        let resumable =
            backend::needs_session(&profile.command) && profile.claude_session_id.is_some();

        if !resumable {
            // shell이거나 sid 없는 claude → 이어받기가 아니라 새 세션 시작(Started).
            return match self.spawn_agent(profile, SpawnMode::Fresh) {
                Ok(_) => RestoreOutcome::Started,
                Err(e) => RestoreOutcome::Failed {
                    reason: e.to_string(),
                },
            };
        }

        // claude resume 시도 → 실패/조기종료면 Failed(시체) 종점(fresh-fallback 폐지, ADR-0082).
        self.resume_no_fallback(profile)
    }

    /// ★resume 전용 공용 규율(ADR-0082 — 부팅복원·수동활성화 공유, fresh-fallback 폐지)★.
    /// 전제: 호출 시점에 이 프로필은 resumable(claude + sid 존재)이라고 이미 판정됐다.
    ///
    /// resume 을 시도하고, spawn 실패거나 EARLY_EXIT_WINDOW 안에 비정상 종료(빈/미대화/손상
    /// 세션이면 claude 가 "No conversation found ..." 로 즉사)하면 **새 대화(fresh)를 자동으로
    /// 만들지 않고** Failed(시체) 종점으로 직행한다 — 사유를 로그로 남겨 LLM 이 읽고 에스컬레이션한다.
    /// ★아무것도 kill·재spawn 하지 않는다★: resume child 는 자기 pump 가 EOF→finish 하고, reaper 가
    ///   그 세션을 맵에서 수거하며 프로필을 `auto_restore=false`(KeepDisableAutoRestore)로 내려
    ///   트리에 `Failed` 시체로 남긴다(profile 은 지워지지 않음 — exit≠0/불명은 삭제 대상이 아님).
    ///   이 헬퍼는 종료를 관측만 하고 어떤 파괴 동작도 하지 않는다(옛 fallback_fresh 의 remove_session·
    ///   epoch++·respawn 을 전부 걷어냈다 — ADR-0082 사용자 결정: "아무것도 죽지마, 새로 만들지마").
    /// 이 로직을 restore_one(부팅 복원)과 activate_profile(수동 활성화)이 **똑같이** 재사용한다.
    // ADR-0082
    // ADR-0008
    fn resume_no_fallback(&self, profile: &AgentProfile) -> RestoreOutcome {
        match self.spawn_agent(profile, SpawnMode::Resume) {
            Err(e) => {
                // resume spawn 자체 실패 — 원인을 로그로 남긴다(삼키면 §5 위반). 새 대화 안 만듦.
                let reason = format!("resume spawn 실패: {e}");
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    "ADR-0082: resume 실패 → Failed(시체), fresh-fallback 없음"
                );
                RestoreOutcome::Failed { reason }
            }
            // ★fable M-1★: 성공한 claude resume은 TUI라 윈도 안에 종료하지 않는다.
            // 따라서 윈도 내 terminal 진입은 code와 무관하게 resume 실패 신호다
            // (code==0 조기 종료를 Resumed로 오판하면 빈 화면을 "복원 성공"으로 오보).
            // None(여전히 Running)만 Resumed.
            Ok(_) => match self.early_terminal_status(profile.id, EARLY_EXIT_WINDOW) {
                Some(status) => {
                    // resume 조기종료 = 이어받을 수 없는 세션(claude "No conversation found ...").
                    // ★원인 로그(제어 표면 입력)★: LLM 에이전트가 이 로그를 읽어 사용자에게
                    //   에스컬레이션한다(ADR-0082 §5). 자동 fresh 대체 없음 — Failed 시체로 남긴다.
                    //   세션은 이미 스스로 종료했으므로 여기서 remove/kill 하지 않는다(reaper 가 수거).
                    let reason = format!("resume 조기 종료({status:?})");
                    tracing::warn!(
                        agent = %profile.id,
                        %reason,
                        "ADR-0082: resume 조기종료 → Failed(시체), fresh-fallback 없음 — LLM 에스컬레이션 대상"
                    );
                    RestoreOutcome::Failed { reason }
                }
                None => RestoreOutcome::Resumed,
            },
        }
    }

    /// spawn 후 window 안에 terminal 상태가 되면 그 상태를, 안 되면 None(여전히 살아있음).
    fn early_terminal_status(&self, id: AgentId, window: Duration) -> Option<AgentStatus> {
        let deadline = Instant::now() + window;
        loop {
            let session = match self.get_session(id) {
                Ok(s) => s,
                // 맵에서 사라짐 = 비정상 → 종료로 간주.
                Err(_) => {
                    return Some(AgentStatus::Failed {
                        message: "session gone".into(),
                    })
                }
            };
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // (옛 remove_session 삭제 — ADR-0082 fresh-fallback 폐지로 유일 호출자 fallback_fresh 가
    //  사라져 dead code 가 됐다. "옛 세션 kill 후 fresh 로 교체" 자체가 폐지된 동작이라 이 silent
    //  cleanup 헬퍼도 함께 제거한다. 정식 kill 은 kill_agent(reaper 위임)가 담당한다.)

    // ── 구독/입출력 ────────────────────────────────────────────────────────

    /// 구독자 등록 + replay 전송 → SinkId. C4 로직은 core.subscribe에 있다.
    pub fn subscribe(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
    ) -> Result<SinkId, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe(sink))
    }

    /// after_seq/epoch resume 구독 → SubscribeOutcome. epoch_matches 는 데몬이 요청 epoch 과
    /// 세션 현재 epoch 을 비교해 넘긴다(코어는 protocol 무의존이라 epoch 비교를 외부에서 받는다).
    pub fn subscribe_from(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> Result<SubscribeOutcome, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe_from(sink, after_seq, epoch_matches, on_ready))
    }

    /// 구독 해제 (창 닫힘 cleanup에서 호출).
    pub fn unsubscribe(&self, agent_id: AgentId, sink_id: SinkId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        session.unsubscribe(sink_id);
        Ok(())
    }

    /// PTY stdin write → transport(Raw 바이트).
    pub fn write_stdin(&self, agent_id: AgentId, data: &[u8]) -> Result<(), PtyError> {
        self.get_session(agent_id)?.write_input(data)
    }

    /// `write_stdin` 의 배달-경계 계측판(ADR-0088 Stage 0) — 성공 시 `WriteOutcome`(논리 메시지 바이트 +
    ///   이 턴의 `msg_uuid`)을 반환한다. 동작은 `write_stdin` 과 동일하고 관측 산출물만 삼키지 않는다.
    ///   제어 채널 relay(ingress::handle_send)가 배달 관측 레코드를 만들 때 쓴다("전송 실패" vs
    ///   "모델 무시" 구별의 전제 — ADR-0088). 완결성 = Ok-vs-Err(바이트 비교 아님)은 `WriteOutcome` 주석 참조.
    pub fn write_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::agent::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.write_input_observed(data)
    }

    /// ★incarnation 조건부 write★ — `expected_epoch` 가 **지금** 그 AgentId 가 가리키는 세션의 epoch 과
    ///   같을 때만 쓴다. 다르면 transport 를 아예 건드리지 않고(부작용 0) `Err` 를 낸다.
    ///
    /// ★왜 필요한가(check-then-write TOCTOU — load-bearing)★: 호출자가 `(id, epoch)` 로 수신자를 정한 뒤
    ///   `write_stdin_observed(id, ..)` 를 부르면, 그 사이 에이전트가 재시작(= 세션 맵 교체 + epoch+1)했을 때
    ///   write 는 **새 incarnation** 에 착지한다. 해석과 write 가 별개 연산인 한 호출자가 아무리 앞서 검사해도
    ///   그 창은 닫히지 않는다 — 판정을 **write 와 같은 단위**로 끌어와야 닫힌다. 그래서 이 함수가 존재한다.
    /// ★★소비자 없음(ADR-0111 결정 6 이후)★★: 이 동사의 **유일한** 호출자는 데몬 메시징의 그룹 방송
    ///   결박(= "발송 순간 살아 있던 그 incarnation 에게만")이었고, **그 불변식은 폐지됐다** — 파킹분은 같은
    ///   이름의 새 화신에게도 배달된다. 그래서 지금 이 함수를 부르는 코드는 없다.
    ///   ★남겨 둔 이유·부활 조건★: "이 편지는 발송 순간 화신에게만" 이 다시 필요해지면 v2 **개인 메일 옵션**
    ///   으로 무파괴 추가하기로 했고(spec §8), 그때 필요한 건 이 조건부 write 하나다. 정식 재론 없이 그룹
    ///   전용 규칙으로 되살리는 것은 ADR-0111 위반이다.
    ///
    /// ★왜 이게 실제로 창을 닫나(ADR-0006 락 규율과 함께 읽을 것)★: `get_session` 은 sessions read lock 을
    ///   잡아 `Arc<AgentSession>` 을 clone 하고 **즉시 해제**한다. 그 뒤의 epoch 비교와 write 는 **같은 Arc**
    ///   위에서 일어나고, `AgentSession.epoch` 는 생성 시 고정되는 불변 필드다(재시작은 세션을 *교체*할 뿐
    ///   기존 세션의 epoch 을 바꾸지 않는다). 따라서 비교 이후 맵이 교체돼도 우리가 쓰는 대상은 바뀔 수
    ///   없다 — 이 함수가 `Ok` 를 내면 "epoch == expected 인 바로 그 세션에 썼다" 가 참이다.
    ///
    /// ★불일치 신호 = `PtyError::Unsupported`(전용 변형을 만들지 않는다)★: 호출자에게 필요한 사실은
    ///   "이 동사를 **지금 이 대상에** 수행할 수 없었고 아무 것도 쓰지 않았다" 하나이고, 그건 이미 있는
    ///   미지원 신호와 같은 모양이다. 원인 특정은 메시지가 담당한다(요구 epoch / 현재 epoch 을 실는다) —
    ///   에러 어휘를 늘리면 이 한 갈래 때문에 모든 호출부의 match 가 넓어진다.
    // ADR-0006
    // ADR-0088
    // ADR-0111 (옛 // ADR-0103 앵커 교체 — 결박 불변식은 폐지됐다. 이 함수는 현재 소비자 없음)
    pub fn write_stdin_observed_if_epoch(
        &self,
        agent_id: AgentId,
        expected_epoch: u32,
        data: &[u8],
    ) -> Result<crate::agent::types::WriteOutcome, PtyError> {
        let session = self.get_session(agent_id)?;
        // ★부작용 0 보장★: 불일치면 transport 를 건드리기 **전에** 빠진다 — 호출자가 이 Err 를 "안 보냈다"
        //   로 확정할 수 있어야 재파킹·skip 판정이 성립한다.
        if session.epoch != expected_epoch {
            return Err(PtyError::Unsupported(format!(
                "epoch mismatch: agent {agent_id} is now at epoch {}, caller required {expected_epoch} — nothing was written",
                session.epoch
            )));
        }
        session.write_input_observed(data)
    }

    /// ★하네스 전용 세션 주입 seam(ADR-0088 / ADR-0012)★ — 미리 조립한 `AgentSession`(테스트 transport
    ///   포함)을 sessions 맵에 직접 등록한다. spawn 파이프(실 PTY·claude 바이너리)를 거치지 않고
    ///   배달-경계 관측 테스트(reachable=structured 캐리어인데 write 성공/실패)를 **바이너리 의존 없이**
    ///   구동하려는 목적이다 — daemon 통합 테스트가 cross-crate 로 봐야 하므로 `test-harness` 기능으로
    ///   게이트한다(`#[doc(hidden)]` 은 접근을 막지 못한다 — 임의 `AgentSession` 주입은 spawn 예약·
    ///   profile+epoch 조율·control-token 발급·pump/reaper 배선·tracker 수명을 통째로 우회하므로,
    ///   운영 빌드에는 아예 컴파일되지 않아야 한다). 기능 OFF = 운영 빌드에 이 메서드 부재. 기능은
    ///   daemon 의 `[dev-dependencies]` 에서만 켜지므로(운영 dep 아님) 운영 daemon 바이너리로 유니피케이션
    ///   되지 않는다. 런타임/운영 경로는 절대 부르지 않는다 — spawn_session 만이 정규 등록점.
    ///
    /// ★안전/불변식★: (a) reaper 미배선 — 주입 세션은 pump 를 start 하지 않으므로 finish hook 이 없고,
    ///   ReapMsg 가 나가지 않아 manager Drop 까지 sessions 맵에 남는다(노출된 remove 없음, `kill_agent` 도
    ///   주입 세션을 맵에서 빼지 않는다 — 각 테스트가 fresh manager 를 쓰고 그 Drop 으로 정리된다).
    ///   (b) 락 규율(ADR-0006) — sessions write lock 을 잡아 insert 후 즉시 해제, 내부 lock 미취득.
    ///   (c) profiles 미터치 — auto_restore 플립·persist 없음(순수 맵 등록). 같은 id 재주입은 교체.
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn insert_test_session(&self, session: Arc<AgentSession>) {
        self.sessions
            .write()
            .expect("sessions poisoned")
            .insert(session.id, session);
    }

    /// PTY cols/rows 변경. resize 성공 시에만 cols/rows atomic 갱신(AgentSession 책임).
    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.get_session(agent_id)?.resize(cols, rows)
    }

    /// 진행 중 작업만 중단(≠kill). PTY=0x03 주입. 프로세스는 살아 있다.
    pub fn interrupt(&self, agent_id: AgentId) -> Result<(), PtyError> {
        self.get_session(agent_id)?.interrupt()
    }

    // ── kill (LLD §6 절대순서 + S9 tracker unwatch) ──────────────────────────

    /// 에이전트 종료 — ★인과 순서 보존 + ADR-0019 reaper 위임★.
    /// intent=UserKill 태깅(shutdown **전**) → enter_exiting(Exiting 알림) → session.kill
    /// (transport.shutdown → master drop → pump EOF → core.finish(Killed)+finish hook→ReapMsg
    /// → join_pump). **맵 제거·disposition·통지는 하지 않는다** — pump 가 보낸 ReapMsg 를 reaper 가
    /// 단일 소비해 처리한다(done 단일 소비자). tracker unwatch 만 직접(reaper 는 tracker 를 모름).
    ///
    /// 의미 변경: 맵 제거가 reaper(비동기)로 옮겨졌다. kill_agent 반환 직후엔 아직 맵에 있을 수
    /// 있으므로, 호출자가 "사라짐"을 단언하려면 폴링해야 한다(headless 테스트가 그렇게 한다).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        // 대상 세션 epoch 을 Arc clone 직후(락 해제 상태) 확정한다 — revoke 대상 (AgentId,epoch).
        let epoch = session.epoch;

        // 0. ★제어 채널 토큰 즉시 폐기 — 블로킹 kill **전에**(FIX 4)★. get_session 이 Arc 를 clone 하고
        //    sessions read lock 을 이미 해제했으므로(§10), 여기서 revoke 를 불러도 락 보유 중이 아니다
        //    (ADR-0006 — registry 는 leaf lock, sessions/status 락 미보유). 예전엔 이 revoke 가
        //    session.kill(최대 5s join) **뒤**라, 죽어가는 에이전트의 토큰이 그 5s 창 동안 유효했다 —
        //    그 사이 에이전트가 제어 채널로 명령을 낼 수 있었다(TOCTOU). 이제 kill 을 시작하기 전에 먼저
        //    폐기해 그 창을 없앤다. revoke 는 idempotent(remove-if-present)라 아래 pump/reaper 의
        //    terminal revoke 와 겹쳐도 무해(그게 backstop). 산 세션이므로 이 epoch 토큰이 지금 폐기 대상.
        // ADR-0086
        self.control.revoke(agent_id, epoch);

        // 0.1. ★intent 태깅을 shutdown 전에★ — finish hook 이 finish 순간 snapshot 하므로, shutdown
        //    이 pump 를 깨워 finish 하기 전에 UserKill 이 보여야 reaper 가 DeleteProfile 로 분류한다.
        session.set_intent(TerminationIntent::UserKill);

        // 0.5. 과도기 Exiting 전이 — kill 누르면 즉시 '종료중' 알림. 전이+발행은 core 안에서
        //      이뤄진다(manager가 트리거, core가 status_changed(Exiting) 발행). 이미 terminal이면
        //      false 반환하나 별도 처리 없음(개별 status_changed(Killed)는 pump의 finish 단독).
        let _ = session.enter_exiting();

        // 1~6. 자원 강제 종료 + pump 완료 대기. shutdown이 master를 drop해 pump read를 EOF로
        //       깨우고(→core.finish(Killed)+hook→ReapMsg), join_pump가 그 pump 종료를 5s 대기한다.
        //       timeout이면 그냥 진행(세션 제거로 Arc 끊겨 자연 종료). ★revoke 배치가 이 인과를 건드리지
        //       않는다(ADR-0001)★: revoke 는 registry/파일만 만지고 shutdown 체인(child.kill→master
        //       drop→pump EOF→finish)에 개입하지 않는다 — kill 을 블록/재정렬하지 않는다.
        session.kill(Duration::from_secs(5));

        // 7. 세션 추적 해제(S9 — 좀비 watcher 엔트리 방지). 맵 제거·통지는 reaper 가 한다.
        //    (제어 채널 revoke 는 위 0단계에서 선제 완료 — reaper terminal revoke 가 idempotent backstop.)
        self.tracker.unwatch(agent_id);

        Ok(())
    }

    // ── 조회/종료 ─────────────────────────────────────────────────────────────

    /// 전체 목록 스냅샷.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        sessions.iter().map(|s| self.agent_info(s)).collect()
    }

    /// replay 스냅샷 조회.
    pub fn get_snapshot(&self, agent_id: AgentId) -> Result<Vec<OutputChunk>, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.snapshot())
    }

    /// 단일 에이전트의 현재 epoch 경량 조회(없으면 None). list_agents 전체 순회·AgentInfo
    /// 조립(profiles lock 등)을 피해 epoch 만 본다 — handle_subscribe 의 epoch_matches 계산용.
    pub fn agent_epoch(&self, agent_id: AgentId) -> Option<u32> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .map(|s| s.epoch)
    }

    /// 앱 종료 시 전체 정리. id를 먼저 모아 sessions lock을 풀고, 각 kill을 병렬 실행한다.
    pub fn shutdown_all(&self) {
        // ★ADR-0019★: shutting_down 을 각 kill **전에** set 한다. 이게 kill 보다 늦으면 그 틈에
        //   종료된 세션의 finish hook 이 shutting_down=false 를 snapshot 해 크래시/유저kill 로
        //   오분류(disposition 적용 → 부팅 복원 대상에서 탈락)하는 race 가 생긴다. set 이 먼저면
        //   이 시점 이후 모든 finish 가 shutting_down=true 를 snapshot → reaper 가 KeepAsIs(손 안 댐).
        self.shutting_down.store(true, Ordering::SeqCst);

        // S9: 세션 추적 스레드부터 정지(폴링이 정리 중인 세션을 건드리지 않게).
        self.tracker.stop();

        let ids: Vec<AgentId> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.keys().copied().collect()
        };
        std::thread::scope(|s| {
            for id in ids {
                s.spawn(move || {
                    let _ = self.kill_agent(id);
                });
            }
        });
    }

    // ── 내부 헬퍼 ─────────────────────────────────────────────

    /// sessions에서 Arc<AgentSession>을 clone해 반환(§10 규칙1: read lock 즉시 해제).
    fn get_session(&self, agent_id: AgentId) -> Result<Arc<AgentSession>, PtyError> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .cloned()
            .ok_or(PtyError::NotFound(agent_id))
    }

    /// id 로 세션을 찾아 AgentInfo 를 조립(없으면 NotFound). activate_profile 이 resume 성공 후
    /// 살아있는 세션의 info 를 얻는 데 쓴다 — resume_no_fallback 은 세션을 맵에 등록만 하고 info 를
    /// 돌려주지 않으므로(RestoreOutcome 반환) id 로 재조회한다. §10 락 순서 준수(get_session 이 read
    /// lock 즉시 해제 → agent_info 는 lock 미보유 상태에서 호출).
    fn agent_info_by_id(&self, id: AgentId) -> Result<AgentInfo, PtyError> {
        let session = self.get_session(id)?;
        Ok(self.agent_info(&session))
    }

    /// id 로 canonical 표시명만 조회(없으면 None). 봉투 sender 등 AgentInfo 전체가 필요 없는
    /// 호출부(daemon ingress::sender_display_name)가 **agent_info 와 byte-identical** 한 이름을
    /// 얻게 하는 단일 출처다 — session.cwd 기반 resolve 를 여기 한 곳에 모아 로직 복제를 막는다.
    /// §10 락 순서: get_session 이 read lock 을 즉시 해제 → resolve 는 lock 미보유에서 수행.
    // ADR-0101
    pub fn canonical_name(&self, id: AgentId) -> Option<String> {
        let session = self.get_session(id).ok()?;
        Some(self.resolve_canonical_name(&session))
    }

    /// session → canonical 표시명(display_name ?? basename(session.cwd)). agent_info·canonical_name
    /// 공유 코어 — 이름 파생을 한 곳으로 모아 reaper/ingress/cli 와 어긋나지 않게 한다.
    ///
    /// ADR-0101 (WYSIWYA — canonical 이름 통일): AgentInfo.name = "사람이 트리에서 보는 이름"으로
    ///   맞춘다. 예전엔 profile.name(= createClaudeProfile 에 넘긴 full cwd 문자열, 종종 경로)을 그대로
    ///   써서 라우팅/로스터가 기대하는 주소와 트리 표시명(display_name ?? basename(cwd))이 어긋났다.
    ///   라우팅(resolve_recipient)·로스터·봉투 sender·프론트 트리가 **같은 문자열**을 써야 "보이는
    ///   이름으로 지목하면 그 에이전트에게 간다"가 성립한다.
    ///
    /// ★cwd 출처 = session.cwd(profile.cwd 아님)★: 프론트 트리는 `display_name ?? basename(AgentInfo.cwd)`
    ///   로 그리고 AgentInfo.cwd = session.cwd(spawn 시 canonicalize). profile.cwd 는 raw("."·".."·심링크)
    ///   라 여기서 파생하면 basename 이 갈려 트리 표시 ≠ 라우팅 주소가 된다. 그래서 AgentInfo.cwd 와
    ///   **같은 값**(session.cwd)에서 파생한다.
    // ADR-0101
    fn resolve_canonical_name(&self, session: &Arc<AgentSession>) -> String {
        // session.cwd = AgentInfo.cwd 와 동일 출처(canonical). 프론트 basename 규칙과 1:1.
        let cwd = session.cwd.to_string_lossy();
        // get()이 profiles lock을 잡아 clone 후 즉시 해제하므로 sessions lock과 동시에 보유하지 않는다
        //   (§10 락 순서, 이 함수는 sessions lock 미보유 상태에서만 호출).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        // 프로필 부재(ad-hoc / 산 세션에 DeleteProfile) 시에도 트리는 basename(cwd)를 그리므로 여기도
        //   cwd basename 으로 파생해야 트리 ≠ 라우팅 이 안 생긴다. cwd 가 placeholder/빈값을 낼 때만
        //   id 앞 8자로 degrade(blank·경로없음 라벨을 주소로 쓰지 않게).
        crate::agent::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
    }

    /// session 스냅샷 → AgentInfo. (sessions lock을 보유하지 않은 상태에서만 호출)
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        let name = self.resolve_canonical_name(session);
        AgentInfo {
            id: session.id,
            name,
            cwd: session.cwd.to_string_lossy().to_string(),
            status: session.status(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
            epoch: session.epoch,
            // transport 종류별 capability — session.capabilities()가 transport.capabilities()를 위임.
            capabilities: session.capabilities(),
        }
    }
}

impl Drop for AgentManager {
    /// reaper 스레드 정리 — Stop 송신 후 join. manager 의 reaper_tx 가 drop 되면 channel 이
    /// 닫혀 recv 가 Err 로도 끝나지만(이중 안전), 세션들이 보유한 hook 클로저가 reaper_tx clone 을
    /// 들고 있어 그것만으로는 즉시 안 닫힐 수 있다. 명시 Stop 으로 확실히 깨운 뒤 join 한다.
    fn drop(&mut self) {
        // Stop 송신(reaper 가 이미 죽었으면 Err — 무시).
        let _ = self.reaper_tx.send(ReaperCmd::Stop);
        if let Some(handle) = self.reaper_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// harmless 자식(cmd.exe /c echo, 즉시 종료)으로 spec을 만든다 — 실 claude 없이 transport
    /// **선택 로직**만 검증하기 위한 격리 하네스(ADR-0012). transport의 caps로 어느 종류가
    /// 골렸는지 판정한다(spawn한 프로세스는 shutdown으로 정리).
    #[cfg(windows)]
    fn probe_spec() -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo select-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    // ── ADR-0044: manager가 json 모드엔 StdioTransport(구조화 caps)를 고른다 ──
    #[cfg(windows)]
    #[test]
    fn select_transport_json_mode_picks_stdio_structured() {
        let (transport, _pid) =
            select_transport(true, &probe_spec(), DEFAULT_COLS, DEFAULT_ROWS, None)
                .expect("select");
        let caps = transport.capabilities();
        assert!(
            caps.output.structured && !caps.output.terminal_bytes,
            "json 모드 → StdioTransport(structured 출력, 터미널 바이트 아님)"
        );
        assert!(!caps.control.resize, "파이프 resize 불가");
        transport.shutdown();
    }

    // ── 회귀: 터미널 모드는 PtyTransport(터미널 바이트, resize 가능, 구조화 아님) ──
    #[cfg(windows)]
    #[test]
    fn select_transport_terminal_mode_picks_pty() {
        let (transport, _pid) =
            select_transport(false, &probe_spec(), DEFAULT_COLS, DEFAULT_ROWS, None)
                .expect("select");
        let caps = transport.capabilities();
        assert!(
            caps.output.terminal_bytes && !caps.output.structured,
            "터미널 모드 → PtyTransport(터미널 바이트, 구조화 아님)"
        );
        assert!(caps.control.resize, "PTY resize 가능");
        transport.shutdown();
    }

    // ── write_stdin_observed_if_epoch — incarnation 조건부 write(ADR-0103 방송 소급 금지의 마지막 관문) ──
    //
    // ★왜 실 spawn 없이 세션을 맵에 직접 꽂나★: 검증 대상은 "맵이 가리키는 세션의 epoch 과 요구 epoch 을
    //   비교해 write 를 집행/거부하는가" 뿐이라, 실 자식·PTY·claude 바이너리가 전부 무관하다(ADR-0012 격리).
    //   in-crate 테스트라 private `sessions` 에 직접 접근한다 — `insert_test_session`(feature gate) 불요.

    use crate::agent::types::{
        ControlCaps, InputCaps, InputEvent, ModelCaps, OutputCaps, SessionCaps, TransportCaps,
    };
    use crate::persistence::{FilePresetStore, FileProfileStore};

    /// write 바이트만 캡처하는 최소 transport(자식·파이프 없음, pump 미기동).
    struct RecordingTransport {
        written: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for RecordingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.written.lock().expect("written poisoned").push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
    }

    /// 빈 레지스트리(임시 디렉토리 store)로 조립한 manager — 이 테스트는 spawn 을 안 쓴다.
    fn bare_manager() -> AgentManager {
        let tag = uuid::Uuid::new_v4();
        let profiles = Arc::new(crate::agent::profile::ProfileRegistry::new(Arc::new(
            FileProfileStore::new(std::env::temp_dir().join(format!("engram-epoch-w-{tag}"))),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-epoch-w-preset-{tag}")),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::agent::session_tracker::TrackerConfig {
                sessions_dir: None,
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        AgentManager::new(Arc::new(NoopStatus), profiles, presets, tracker)
    }

    /// 주어진 epoch 의 세션을 맵에 꽂는다(같은 id 재삽입 = 재시작 = incarnation 교체 모사).
    fn put_session(manager: &AgentManager, id: AgentId, epoch: u32) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let written = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(id, epoch, Arc::new(NoopStatus)));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            core,
            Box::new(RecordingTransport {
                written: written.clone(),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        written
    }

    #[test]
    fn write_stdin_observed_if_epoch_writes_when_the_incarnation_matches() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let written = put_session(&manager, id, 3);

        let out = manager
            .write_stdin_observed_if_epoch(id, 3, b"hello")
            .expect("일치하면 정상 write");
        assert_eq!(out.bytes_requested, 5);
        assert_eq!(
            out.epoch, 3,
            "WriteOutcome.epoch = write 를 집행한 세션의 epoch"
        );
        assert_eq!(
            written.lock().unwrap().as_slice(),
            &[b"hello".to_vec()],
            "요구 epoch 과 현재 incarnation 이 같으면 그대로 쓴다"
        );
    }

    #[test]
    fn write_stdin_observed_if_epoch_refuses_a_replaced_incarnation_without_writing() {
        // ★핵심 회귀★: 호출자가 epoch 0 을 보고 결정한 뒤 그 사이 재시작(epoch 1 교체)이 일어난 상황.
        //   무조건 write 하는 옛 경로는 **새 incarnation** 에 착지했다(방송 소급 배달 — ADR-0103 위반).
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let old_written = put_session(&manager, id, 0);
        // 재시작 = 같은 AgentId 를 **새 세션**(epoch 1)으로 교체.
        let new_written = put_session(&manager, id, 1);

        let err = manager
            .write_stdin_observed_if_epoch(id, 0, b"broadcast")
            .expect_err("교체된 incarnation 에는 쓰지 않는다");
        assert!(
            matches!(err, PtyError::Unsupported(ref m) if m.contains("epoch mismatch")),
            "불일치는 미지원 신호 + 원인 메시지: {err}"
        );
        assert!(
            new_written.lock().unwrap().is_empty(),
            "새 incarnation 에 단 한 바이트도 가면 안 된다(부작용 0)"
        );
        assert!(
            old_written.lock().unwrap().is_empty(),
            "옛 세션은 맵에서 밀려났으므로 그쪽에도 쓰지 않는다"
        );
        // 대조: 요구 epoch 을 현재 값으로 맞추면 그대로 쓴다(거부가 '영구 봉쇄'가 아님).
        manager
            .write_stdin_observed_if_epoch(id, 1, b"broadcast")
            .expect("현재 incarnation 지목은 통과");
        assert_eq!(new_written.lock().unwrap().len(), 1);
    }

    #[test]
    fn write_stdin_observed_if_epoch_reports_not_found_for_an_unknown_agent() {
        let manager = bare_manager();
        let err = manager
            .write_stdin_observed_if_epoch(AgentId::new_v4(), 0, b"x")
            .expect_err("없는 에이전트");
        assert!(
            matches!(err, PtyError::NotFound(_)),
            "부재는 epoch 불일치와 다른 사실이다: {err}"
        );
    }

    // ── 명부 단일 입구(ADR-0119) · 이름 전역 유일(ADR-0120) ─────────────────────

    /// 지정 cwd 로 **산 세션**을 맵에 꽂는다(프로필 없음 = ad-hoc 산 에이전트). `put_session` 은 cwd 가
    /// `"."` 로 고정이라 이름 축 단언을 못 해서 따로 둔다 — 이름은 `basename(session.cwd)` 로 파생된다.
    fn put_live_session_at(manager: &AgentManager, id: AgentId, cwd: &str) {
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatus)));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(cwd),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            core,
            Box::new(RecordingTransport {
                written: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
    }

    /// 잠들 프로필 하나(셸 명령 — spawn 하지 않는 테스트에선 실제로 실행되지 않는다).
    fn agent_profile(cwd: &str, display_name: Option<&str>) -> AgentProfile {
        let mut p = AgentProfile::new(
            "raw".into(),
            crate::agent::profile::AgentCommand::Shell {
                program: default_shell().to_string(),
                args: vec![],
            },
            std::path::PathBuf::from(cwd),
            vec![],
            false,
        );
        p.display_name = display_name.map(|s| s.to_string());
        p
    }

    /// 등록 성공을 전제로 하는 편의 래퍼(접미사 공간 소진만이 Err 이고 대부분 테스트는 그 경로가 아니다).
    fn create(manager: &AgentManager, cwd: &str, display_name: Option<&str>) -> AgentProfile {
        manager
            .create_agent(agent_profile(cwd, display_name))
            .expect("이 픽스처는 접미사 공간을 소진시키지 않는다")
    }

    /// 개명 요청이 성공했는지(확정 또는 멱등 무변경). 실패 두 갈래는 각각 별도 테스트가 구분한다.
    fn renamed_ok(o: RenameOutcome) -> bool {
        matches!(o, RenameOutcome::Renamed(_) | RenameOutcome::Unchanged(_))
    }

    /// 명부(로스터)가 그 에이전트에 대해 말하는 canonical 이름 — 산 에이전트면 session.cwd 축이다.
    fn name_of_in_roster(manager: &AgentManager, id: AgentId) -> String {
        manager
            .roster()
            .into_iter()
            .find(|e| e.id == id)
            .expect("명부에 있어야")
            .canonical_name
    }

    /// 명부에 저장된 그 에이전트의 현재 canonical 이름.
    fn name_of(manager: &AgentManager, id: AgentId) -> String {
        manager
            .agent_snapshot(id)
            .expect("명부에 있어야")
            .canonical_name_when_live()
    }

    #[test]
    fn roster_reports_live_and_dormant_agents_in_one_query() {
        // ★새 계약(ADR-0119 결정 2)★: "전체 에이전트 + 각자 생사" 를 **한 번의 조회**가 준다.
        //   소비자가 산 목록과 프로필 목록을 각자 떠서 합치면 그 사본이 drift 한다(이 ADR 의 발생 원인).
        let manager = bare_manager();
        let live_id = AgentId::new_v4();
        put_live_session_at(&manager, live_id, "C:/roster/alpha");
        let dormant = create(&manager, "C:/roster/beta", Some("beta"));

        let roster = manager.roster();
        assert_eq!(roster.len(), 2, "산 1 + 잠듦 1: {roster:?}");
        let live = roster
            .iter()
            .find(|e| e.id == live_id)
            .expect("산 에이전트가 명부에 있어야");
        assert!(live.live.is_some(), "산 항목은 세션이 붙어 있어야");
        assert_eq!(
            live.canonical_name, "alpha",
            "산 이름은 AgentInfo(session.cwd 기반)에서 온다"
        );
        let asleep = roster
            .iter()
            .find(|e| e.id == dormant.id)
            .expect("잠든 에이전트도 같은 명부에 있어야");
        assert!(asleep.live.is_none(), "잠든 항목엔 세션이 없다");
        assert_eq!(
            asleep.canonical_name, "beta",
            "잠든 이름은 canonical_name_when_live() 에서 온다"
        );
    }

    #[test]
    fn a_live_namesake_does_not_hide_a_dormant_agent_with_the_same_name() {
        // ★봉인 대상 = 잠듦 차집합의 축이 **id** 라는 규칙 자체★. 이름 축으로 빼면 산 동명 하나가
        //   잠든 다른 에이전트를 통째로 가려, 그 앞으로 온 편지가 주인을 못 만난다.
        let manager = bare_manager();
        let live_id = AgentId::new_v4();
        // 산 이름 = "twin"(프로필 없는 ad-hoc 세션이라 basename(session.cwd) 파생).
        put_live_session_at(&manager, live_id, "C:/roster/twin");
        // ★유일성을 우회해 직접 심는다★: 정상 경로(create_agent)면 ADR-0120 이 "twin(1)" 로 개명하므로
        //   이 상태를 만들 수 없다. 여기서 보는 건 그 위층 규칙이 아니라 **차집합의 축**이다.
        let dormant = agent_profile("C:/elsewhere/quiet", Some("twin"));
        let dormant_id = dormant.id;
        manager.profiles.upsert(dormant);

        let roster = manager.roster();
        assert_eq!(
            roster
                .iter()
                .filter(|e| e.canonical_name == "twin")
                .count(),
            2,
            "산 동명과 잠든 동명이 **둘 다** 있어야(이름 축으로 빼면 잠든 쪽이 사라진다): {roster:?}"
        );
        assert!(
            roster
                .iter()
                .any(|e| e.id == dormant_id && e.live.is_none()),
            "잠든 쪽은 id 가 산 집합에 없으므로 남아야: {roster:?}"
        );
    }

    #[test]
    fn two_dormant_agents_sharing_a_name_are_both_reported() {
        // 동명 잠듦은 **접지 않는다** — dedup 하면 동명 판정(모호 반려)이 조용히 파킹으로 바뀐다.
        let manager = bare_manager();
        manager.profiles.upsert(agent_profile("C:/a", Some("twin")));
        manager.profiles.upsert(agent_profile("C:/b", Some("twin")));

        let roster = manager.roster();
        assert_eq!(
            roster
                .iter()
                .filter(|e| e.canonical_name == "twin" && e.live.is_none())
                .count(),
            2,
            "동명 잠듦 2건은 2건 그대로: {roster:?}"
        );
    }

    #[test]
    fn a_dormant_agent_with_a_display_name_does_not_depend_on_the_filesystem() {
        // ★발송 임계 경로 보호(canonical_name_when_live 의 override 단축)★: override 가 있으면 이름이
        //   fs 에 전혀 의존하지 않아야 한다. 명부가 cwd 를 선제 canonicalize 하면 죽은 네트워크 공유에서
        //   발송 1회당 수십 초가 붙고, cwd 가 사라진 프로필은 이름이 basename 으로 흔들린다.
        // ★관측 방법의 한계(정직 명시)★: syscall 자체는 세지 못한다 — 실재하지 않는 cwd 를 써서
        //   "canonicalize 를 탔다면 결과가 달라졌을" 상황을 만들고 결과 불변을 단언한다.
        let vanished = "C:/engram-does-not-exist-9f1c/never/created";
        assert!(
            dunce::canonicalize(vanished).is_err(),
            "이 테스트의 전제 — 이 경로는 실재하지 않아야 한다"
        );
        let manager = bare_manager();
        let p = create(&manager, vanished, Some("Named"));

        let roster = manager.roster();
        let entry = roster.iter().find(|e| e.id == p.id).expect("잠듦 항목");
        assert_eq!(
            entry.canonical_name, "Named",
            "override 가 있으면 cwd 파생(=fs 접근)을 타지 않는다 — cwd 를 봤다면 'created' 가 됐을 것"
        );
    }

    #[test]
    fn creating_a_colliding_name_gets_the_next_free_suffix() {
        // ADR-0120: 명부 전체(산+잠듦)에서 canonical 이름은 유일하다. 충돌은 거부가 아니라 자동 접미사.
        let manager = bare_manager();
        let a = create(&manager, "C:/x", Some("bob"));
        assert_eq!(a.canonical_name_when_live(), "bob", "첫 번째는 그대로");
        let b = create(&manager, "C:/x", Some("bob"));
        assert_eq!(b.canonical_name_when_live(), "bob(1)");
        let c = create(&manager, "C:/x", Some("bob"));
        assert_eq!(
            c.canonical_name_when_live(),
            "bob(2)",
            "번호는 현재 계열 최대 + 1"
        );
    }

    #[test]
    fn a_suffix_number_is_reused_once_nothing_holds_it() {
        // 번호 기준은 **지금 명부에 남아 있는 것**뿐이므로(ADR-0123) `bob(1)` 을 지우면 다음 `bob` 충돌은
        //   다시 `bob(1)` 이다.
        // ★안전 근거★: 프로필 삭제는 메시징 삭제 정리 훅을 돌리므로(connection_core DeleteProfile) 재발급된
        //   이름이 옛 주인의 파킹 메일·오픈 계약을 물려받지 않는다.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let one = create(&manager, "C:/x", Some("bob"));
        assert_eq!(one.canonical_name_when_live(), "bob(1)");
        manager.delete_agent(one.id);
        let again = create(&manager, "C:/x", Some("bob"));
        assert_eq!(
            again.canonical_name_when_live(),
            "bob(1)",
            "그 번호를 쥔 것이 아무도 없으면 다시 (1) 이다"
        );
    }

    #[test]
    fn renaming_into_an_existing_name_gets_a_suffix_but_self_rename_does_not() {
        // ADR-0120 결정 2: 검사 지점에 **개명**이 포함된다(0115 는 신규 등록만 봤다).
        let manager = bare_manager();
        let bob = create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert!(renamed_ok(
            manager.rename_agent(alice.id, Some("bob".into()))
        ));
        assert_eq!(
            name_of(&manager, alice.id),
            "bob(1)",
            "남의 이름으로 개명하면 접미사가 붙는다"
        );
        // 자기 자신은 충돌 계산에서 빠진다 — 같은 이름 재확정은 no-op 이어야 한다.
        assert!(renamed_ok(manager.rename_agent(bob.id, Some("bob".into()))));
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "자기 이름 재확정에 접미사가 붙으면 개명할 때마다 번호가 늘어난다"
        );
    }

    #[test]
    fn repeating_a_rename_request_does_not_burn_a_new_number() {
        // ★개명 멱등성(이름 = 메일 주소, ADR-0116)★: B 를 `bob` 으로 개명하면 `bob(1)` 이 되는데, 사용자는 "안 먹었나" 싶어
        //   같은 요청을 다시 낸다. 그때마다 번호가 타면 `bob(2)`·`bob(3)` … 로 **메일 주소가 계속 바뀌어**
        //   직전 이름으로 파킹된 편지가 24h TTL 까지 고아가 된다(ADR-0116 이 막으려는 결말).
        //   프론트의 "값 안 바뀜" 가드는 현재 이름이 `bob(1)` 이라 걸리지 않고, LLM `RenameProfile` 엔 가드가 없다.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert!(renamed_ok(
            manager.rename_agent(alice.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, alice.id), "bob(1)");
        // 같은 요청 재제출 ×2 — 이미 그 계열의 유일 이름을 쥐고 있으므로 완전 no-op.
        assert!(
            renamed_ok(manager.rename_agent(alice.id, Some("bob".into()))),
            "재요청은 실패가 아니라 성공(no-op)으로 보고한다"
        );
        assert_eq!(
            name_of(&manager, alice.id),
            "bob(1)",
            "번호를 태우지 않는다"
        );
        assert!(renamed_ok(
            manager.rename_agent(alice.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, alice.id), "bob(1)");
        // 명부 어디에도 bob(2) 가 생기지 않았다.
        assert!(
            !manager
                .roster()
                .iter()
                .any(|e| e.canonical_name == "bob(2)"),
            "재요청이 새 번호를 만들면 안 된다: {:?}",
            manager.roster()
        );

        // ★★낮은 번호가 비었을 때가 진짜 시험대★★:
        //   재요청은 자기 자신을 충돌 계산에서 빼므로, **낮은 번호가 비어 있으면** 단축이 없을 때
        //   재요청이 에이전트를 `bob(3)` → `bob(1)` 로 **끌어내린다**. 이름이 바뀌는 순간 그게 곧 메일
        //   주소 변경이고 직전 이름으로 파킹된 편지는 24h TTL 까지 고아가 된다(ADR-0116).
        //   그래서 계약은 "결과가 우연히 같다" 가 아니라 "아무것도 하지 않는다" 다.
        let filler = create(&manager, "C:/f", Some("bob")); // alice 가 bob(1) 이므로 bob(2)
        assert_eq!(filler.canonical_name_when_live(), "bob(2)");
        let carol = create(&manager, "C:/c", Some("carol"));
        assert!(renamed_ok(
            manager.rename_agent(carol.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, carol.id), "bob(3)");
        manager.delete_agent(filler.id); // bob(2) 가 비었다 — 낮은 번호가 열린 상태
        assert!(
            renamed_ok(manager.rename_agent(carol.id, Some("bob".into()))),
            "같은 요청 재제출"
        );
        assert_eq!(
            name_of(&manager, carol.id),
            "bob(3)",
            "재요청은 빈 낮은 번호로 끌어내리지 않는다(이름=주소가 흔들린다)"
        );
    }

    #[test]
    fn clearing_an_override_into_a_collision_suffixes_and_is_idempotent() {
        // ★override 해제도 개명이다(ADR-0120 결정 2)★: 해제하면 canonical 이름이 cwd basename 으로 바뀌므로
        //   그 결과가 남의 이름과 겹칠 수 있다. 검사를 빼면 동명 2건이 명부에 앉는다.
        let manager = bare_manager();
        let a = create(&manager, "C:/shared", None);
        assert_eq!(
            a.canonical_name_when_live(),
            "shared",
            "override 없으면 cwd basename 파생(이 테스트의 전제)"
        );
        let b = create(&manager, "C:/shared", Some("bee"));
        assert_eq!(b.canonical_name_when_live(), "bee");

        // 해제 → cwd 파생 이름이 A 와 충돌 → 접미사 붙은 override 가 남는다(동명 허용의 대안).
        assert!(renamed_ok(manager.rename_agent(b.id, None)));
        assert_eq!(name_of(&manager, b.id), "shared(1)");
        assert_eq!(
            manager.agent_snapshot(b.id).unwrap().display_name,
            Some("shared(1)".to_string()),
            "충돌하는 해제는 override 를 없애지 않는다(없애면 동명이 된다)"
        );
        // 같은 해제 요청 재제출 — 이미 그 계열의 유일 이름을 쥐고 있으므로 번호를 태우지 않는다.
        assert!(renamed_ok(manager.rename_agent(b.id, None)));
        assert_eq!(name_of(&manager, b.id), "shared(1)", "해제 재요청도 멱등");
        // A 는 그대로 "shared" 를 쥐고 있어야 한다(해제가 남의 이름을 흔들지 않는다).
        assert_eq!(name_of(&manager, a.id), "shared");
    }

    #[test]
    fn a_literal_zero_suffix_does_not_occupy_the_unsuffixed_name() {
        // ★`Exact`(접미사 없음)와 `Suffixed(0)`(리터럴 `bob(0)`)을 한 값으로 섞으면★,
        //   `bob(0)` 하나가 `bob` 을 점유한 것처럼 보여 `bob` 이 비었는데도 `bob(1)` 이 발급된다.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob(0)"));

        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "리터럴 bob(0) 은 접미사 없는 bob 을 점유하지 않는다"
        );
        // 그리고 bob 이 실제로 차면 계열 최대(bob(0) → 0) + 1 = bob(1).
        let next = create(&manager, "C:/z", Some("bob"));
        assert_eq!(next.canonical_name_when_live(), "bob(1)");
    }

    #[test]
    fn the_requested_name_is_the_base_verbatim() {
        // ★문서화돼 있었으나 봉인은 없던 계약★: base 는 요청 문자열 **그대로**다 — 접미사를 벗겨
        //   다른 계열로 재해석하지 않는다. 그래서 `bob(1)` 요청은 `bob` 계열과 무관하다.
        let manager = bare_manager();
        let one = create(&manager, "C:/x", Some("bob(1)"));
        assert_eq!(
            one.canonical_name_when_live(),
            "bob(1)",
            "비어 있으면 요청한 이름 그대로(계열로 재해석 금지)"
        );
        // ★`bob` 은 여전히 비어 있다★: `bob(1)` 은 `Suffixed(1)` 이라 `bob` 의 Exact 점유가 아니다.
        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "bob(1) 이 있다고 bob 을 못 쓰게 되면 삭제로 이름을 회수하는 경로가 막힌다"
        );
        // 같은 base 를 또 요청하면 중첩 표기가 된다(벗기지 않는다는 계약의 귀결).
        let nested = create(&manager, "C:/z", Some("bob(1)"));
        assert_eq!(nested.canonical_name_when_live(), "bob(1)(1)");
        // ★파서가 `bob(1)(1)` 을 `bob` 계열로 되읽지 않는다★: 되읽으면 아래가 bob(2) 가 아니라 다른 번호가 된다.
        let bob2 = create(&manager, "C:/w", Some("bob"));
        assert_eq!(
            bob2.canonical_name_when_live(),
            "bob(2)",
            "bob 계열 최대는 bob(1) 의 1 뿐이다(bob(1)(1) 은 계열 아님)"
        );
        // 중첩 계열도 자기 base 로 정상 증가한다.
        let nested2 = create(&manager, "C:/v", Some("bob(1)"));
        assert_eq!(nested2.canonical_name_when_live(), "bob(1)(2)");
    }

    #[test]
    fn a_saturated_family_falls_back_to_the_lowest_free_number() {
        // ★계열 최대가 `u32::MAX` 여도 배정은 죽지 않는다★: "최대 + 1" 이 없으니 포화 탈출구(가장 낮은
        //   빈 번호)로 내려간다. 산술 오버플로로 게이트 안에서 패닉하면 그 Mutex 가 poison 돼 생성·개명·
        //   스폰이 데몬 재시작까지 막히고, 반대로 거부로 처리하면 그 계열의 42억 개 빈 번호가 영구 봉쇄된다
        //   (`이름(4294967295)` 은 UI 개명 한 번으로 만들 수 있는 평범한 상태다).
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        // u32::MAX 접미사를 **리터럴 이름**으로 가진 에이전트.
        create(&manager, "C:/y", Some("bob(4294967295)"));

        // 최대 + 1 이 불가능 → 가장 낮은 빈 번호(1).
        let first = create(&manager, "C:/z", Some("bob"));
        assert_eq!(
            first.canonical_name_when_live(),
            "bob(1)",
            "포화여도 빈 번호를 준다(거부하면 계열 전체가 영구 봉쇄된다)"
        );
        // 1 이 차면 다음 구멍은 2.
        let second = create(&manager, "C:/w", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(2)");

        // 개명도 같은 탈출구를 탄다.
        let carol = create(&manager, "C:/c", Some("carol"));
        assert!(renamed_ok(
            manager.rename_agent(carol.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, carol.id), "bob(3)");

        // ★게이트가 poison 되지 않았다★ — 무관한 이름 배정이 계속 된다.
        let fine = create(&manager, "C:/ok", Some("dave"));
        assert_eq!(fine.canonical_name_when_live(), "dave");
    }

    #[test]
    fn pick_suffix_walks_up_then_fills_the_lowest_hole_only_when_saturated() {
        use std::collections::BTreeSet;
        let set = |v: &[u32]| -> BTreeSet<u32> { v.iter().copied().collect() };

        // 정상 경로 = 관측 최대 + 1(단조 증가 — 산 것끼리 번호가 겹치지 않는다).
        assert_eq!(pick_suffix(&set(&[])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, 2])), Some(3));
        assert_eq!(
            pick_suffix(&set(&[2])),
            Some(3),
            "구멍(1)이 있어도 포화가 아니면 내려가지 않는다"
        );
        // 포화(최대 = MAX)일 때만 가장 낮은 구멍으로.
        assert_eq!(pick_suffix(&set(&[u32::MAX])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, u32::MAX])), Some(2));
        assert_eq!(pick_suffix(&set(&[1, 2, 3, u32::MAX])), Some(4));
        assert_eq!(
            pick_suffix(&set(&[2, u32::MAX])),
            Some(1),
            "포화 갈래는 가장 낮은 구멍을 쓴다"
        );
        // ★`None`(계열 전체 점유)은 테스트로 봉인하지 않는다★: 1..=u32::MAX 를 다 채운 입력을 만들 수
        //   없다(42억 엔트리). 전역 함수를 종결시키기 위한 갈래로만 존재한다.
    }
    #[test]
    fn epoch_replacement_never_renames_an_existing_agent() {
        // ★★이 슬라이스에서 가장 비싼 실수★★: restart/restore/재활성화는 **같은 AgentId 의 맵 교체**
        //   (ADR-0007)지 신규 등록이 아니다. 여기서 유일성을 걸면 재시작할 때마다 이름에 (1) 이 붙는다.
        let manager = bare_manager();
        let bob = create(&manager, "C:/x", Some("bob"));

        // 신규 등록(명부에 없는 id) — 접미사가 붙어야 한다.
        let newcomer = agent_profile("C:/x", Some("bob"));
        manager
            .register_for_spawn(&newcomer)
            .expect("신규 등록 성공");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "명부에 없던 id = 신규 등록 → 접미사"
        );

        // 같은 id 재등록(= epoch 교체) — 이름 불변.
        manager.register_for_spawn(&bob).expect("재등록 성공");
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "재시작이 이름을 바꾸면 안 된다"
        );
        // 접미사를 받은 쪽도 재등록에 다시 붙지 않는다(bob(1)(1) 금지). 넘기는 건 개명 전 stale
        //   스냅샷이지만 upsert_preserving_hierarchy 가 live display_name 을 보존한다(ADR-0070).
        manager.register_for_spawn(&newcomer).expect("재등록 성공");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "재등록은 접미사를 누적하지 않는다"
        );

        // 재등록이 계열 번호를 흔들지 않았다 — 다음 신규는 bob(2) 다.
        let third = create(&manager, "C:/x", Some("bob"));
        assert_eq!(third.canonical_name_when_live(), "bob(2)");

        // ★★stale 스냅샷 재등록 — 실전 회귀 시나리오★★: 산 세션 도중 트리에서 개명 → 그 뒤 재시작이
        //   **개명 전 스냅샷**을 들고 재등록하는데, 그 옛 이름을 그사이 **다른 에이전트가 차지**했다.
        //   이때 신규-등록 검사가 돌면 옛 이름이 충돌로 판정돼 재시작이 에이전트를 엉뚱한 이름으로 개명한다.
        //   (`upsert_preserving_hierarchy` 의 live 보존이 2중 방어이므로, 이름이 실제로 새는 것은 신규-등록
        //   분기 제거와 그 보존 규칙 상실이 **함께** 일어날 때다.)
        let agent = create(&manager, "C:/stale", Some("was-here"));
        let stale_snapshot = agent.clone(); // display_name = "was-here"
        assert!(renamed_ok(
            manager.rename_agent(agent.id, Some("renamed".into()))
        ));
        assert_eq!(name_of(&manager, agent.id), "renamed");
        // 옛 이름을 다른 에이전트가 가져간다(개명으로 비었으므로 접미사 없이 그대로 얻는다).
        let squatter = create(&manager, "C:/squat", Some("was-here"));
        assert_eq!(squatter.canonical_name_when_live(), "was-here");
        // 재시작 = 옛 스냅샷으로 재등록. 이름은 절대 흔들리지 않아야 한다.
        manager
            .register_for_spawn(&stale_snapshot)
            .expect("재등록 성공");
        assert_eq!(
            name_of(&manager, agent.id),
            "renamed",
            "재시작이 stale 스냅샷의 옛 이름으로 개명 판정을 하면 안 된다"
        );
        assert_eq!(
            name_of(&manager, squatter.id),
            "was-here",
            "재시작이 남의 이름도 건드리지 않는다"
        );
    }

    #[test]
    fn renaming_into_a_freed_name_actually_takes_it() {
        // ★★결정표 3 — "아무도 안 갖고 있으면 준다"★★. 이 갈래가 없으면 **조용한 성공**이 된다:
        //   계열 판정을 먼저 걸러 버리면 `bob(1)` 을 쥔 에이전트는 `bob` 이 비어 있어도 개명되지 않는데
        //   호출부는 Ack + 목록 broadcast 까지 해서, 사용자와 LLM 둘 다 "됐다" 고 본다(안 된 일을).
        let manager = bare_manager();
        let first = create(&manager, "C:/x", Some("bob"));
        let second = create(&manager, "C:/y", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(1)");

        // `bob` 을 비운다 — 이제 요청 이름의 주인이 아무도 없다.
        manager.delete_agent(first.id);
        let out = manager.rename_agent(second.id, Some("bob".into()));
        assert_eq!(
            out,
            RenameOutcome::Renamed("bob".to_string()),
            "빈 이름 요청은 확정돼야 한다(무변경 성공 보고는 거짓이다)"
        );
        assert_eq!(name_of(&manager, second.id), "bob");
    }

    #[test]
    fn clearing_an_override_works_once_the_derived_name_is_free() {
        // ★같은 결정표 3 — override 해제 갈래★. 계열 판정을 먼저 걸면 해제가 **영구 불가**가 된다:
        //   `C:/shared` 의 `shared(1)` 은 해제 결과(`shared`)가 늘 제 계열이라 매번 no-op 으로 걸린다.
        let manager = bare_manager();
        let holder = create(&manager, "C:/shared", None);
        assert_eq!(holder.canonical_name_when_live(), "shared");
        let b = create(&manager, "C:/shared", Some("bee"));
        assert!(renamed_ok(manager.rename_agent(b.id, None)));
        assert_eq!(
            name_of(&manager, b.id),
            "shared(1)",
            "충돌 중엔 접미사 유지"
        );

        // 주인이 사라지면 해제가 실제로 override 를 지워야 한다.
        manager.delete_agent(holder.id);
        let out = manager.rename_agent(b.id, None);
        assert_eq!(out, RenameOutcome::Renamed("shared".to_string()));
        assert_eq!(
            manager.agent_snapshot(b.id).unwrap().display_name,
            None,
            "해제가 가능해졌으면 override 가 실제로 지워져야 한다"
        );
        assert_eq!(name_of(&manager, b.id), "shared");
    }

    #[test]
    fn rename_failures_are_distinguishable_from_each_other() {
        // ★bool 이면 "그런 에이전트가 없다" 와 "이름을 발급할 수 없다" 가 한 값으로 뭉개져 wire 응답이
        //   거짓 원인을 말한다★. 성공 두 갈래도 서로 구분돼야 한다(확정 vs 멱등 무변경).
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert_eq!(
            manager.rename_agent(AgentId::new_v4(), Some("bob".into())),
            RenameOutcome::NotFound,
            "없는 id 는 NotFound 다(이름 문제가 아니다)"
        );
        assert_eq!(
            manager.rename_agent(alice.id, Some("bob".into())),
            RenameOutcome::Renamed("bob(1)".to_string()),
            "확정은 확정된 이름을 함께 보고한다"
        );
        assert_eq!(
            manager.rename_agent(alice.id, Some("bob".into())),
            RenameOutcome::Unchanged("bob(1)".to_string()),
            "멱등 재요청은 무변경으로 구분된다"
        );
    }

    #[test]
    fn a_live_agents_own_name_is_read_from_the_roster_not_re_derived() {
        // ★산 에이전트의 이름은 session.cwd 기반, 프로필 파생은 profile.cwd 기반이라 갈릴 수 있다★
        //   (`roster()` doc — 두 축은 재료가 다르다). 결정표가 자기 현재 이름을 따로 파생해 비교하면
        //   계열 판정이 틀려, 이미 유일 이름을 쥔 에이전트에 접미사를 새로 발급한다(= 조용한 개명).
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob")); // 남이 `bob` 을 쥔다
                                               // X: 프로필 파생 이름은 "zeta"(profile.cwd), 명부가 말하는 이름은 "bob(1)"(session.cwd).
        let x = create(&manager, "C:/somewhere/zeta", None);
        assert_eq!(x.canonical_name_when_live(), "zeta", "프로필 파생 축");
        put_live_session_at(&manager, x.id, "C:/live/bob(1)");
        assert_eq!(name_of_in_roster(&manager, x.id), "bob(1)", "명부 축");

        // `bob` 은 남이 쥐었고 X 는 이미 그 계열(`bob(1)`)이다 → 멱등 무변경이어야 한다.
        let out = manager.rename_agent(x.id, Some("bob".into()));
        assert_eq!(
            out,
            RenameOutcome::Unchanged("bob(1)".to_string()),
            "명부가 말하는 자기 이름으로 판정해야 한다"
        );
        assert_eq!(
            manager.agent_snapshot(x.id).unwrap().display_name,
            None,
            "무변경이면 override 를 새로 심지 않는다(심으면 조용한 개명이다)"
        );
    }

    #[test]
    fn every_name_gate_entrance_stores_the_override_without_edge_whitespace() {
        // ★한 입구라도 빠지면 그리로 동명이 새어 들어온다★: 유일성은 문자열 비교라 `" bob "` 은 `bob` 과
        //   다른 이름으로 통과하는데 화면엔 똑같이 그려지고, 메시징 입구는 수신자를 trim 해 맞추므로 그렇게
        //   저장된 에이전트는 이름으로 주소 지정도 안 된다.
        // 지금 override 를 싣는 산 경로는 개명 하나뿐이지만(나머지 둘은 `display_name: None` 로 들어온다)
        //   셋 다 공개 API 라 함께 봉인한다 — 나중에 생성·spawn 이 이름을 나르게 되는 날 조용히 뚫린다.
        let manager = bare_manager();

        let created = create(&manager, "C:/x", Some("  bob  "));
        assert_eq!(
            created.display_name,
            Some("bob".to_string()),
            "생성 응답이 들고 가는 값부터 정규화돼야 한다"
        );
        assert_eq!(
            manager.agent_snapshot(created.id).unwrap().display_name,
            Some("bob".to_string()),
            "명부에 저장된 값도 같아야 한다"
        );

        let renamed = create(&manager, "C:/y", Some("carol"));
        assert!(renamed_ok(
            manager.rename_agent(renamed.id, Some("\tdave\n".into()))
        ));
        assert_eq!(
            manager.agent_snapshot(renamed.id).unwrap().display_name,
            Some("dave".to_string()),
            "개명 경로(탭·개행 포함)"
        );

        // spawn 신규 등록 — 프로필을 그대로 심는 세 번째 입구.
        let spawned = agent_profile("C:/z", Some(" erin "));
        manager
            .register_for_spawn(&spawned)
            .expect("신규 등록 성공");
        assert_eq!(
            manager.agent_snapshot(spawned.id).unwrap().display_name,
            Some("erin".to_string()),
            "신규 등록 경로"
        );
    }

    #[test]
    fn a_whitespace_only_override_is_stored_as_no_override() {
        // 공백-only override 는 파생 함수들이 이미 무시하지만(빈 라벨 방지), 그건 **표시**만 구제할 뿐
        //   명부엔 쓸모없는 문자열이 남는다. 저장 단계에서 없앤 override 로 확정해 cwd 파생으로 떨어뜨린다.
        let manager = bare_manager();

        let blank = create(&manager, "C:/blankdir", Some("   "));
        assert_eq!(
            blank.display_name, None,
            "공백만 남는 요청은 override 없음으로 저장된다"
        );
        assert_eq!(
            name_of(&manager, blank.id),
            "blankdir",
            "override 가 없으니 cwd 파생 이름"
        );

        let named = create(&manager, "C:/otherdir", Some("zoe"));
        assert!(renamed_ok(manager.rename_agent(named.id, Some(" ".into()))));
        assert_eq!(
            manager.agent_snapshot(named.id).unwrap().display_name,
            None,
            "개명 경로의 공백-only 요청 = override 해제"
        );
        assert_eq!(name_of(&manager, named.id), "otherdir");
    }

    #[test]
    fn interior_whitespace_is_part_of_the_name() {
        // 양끝만 깎는다 — 안쪽 공백까지 건드리면 사용자가 지은 이름이 조용히 다른 이름이 된다.
        let manager = bare_manager();
        let a = create(&manager, "C:/x", Some("  bob smith  "));
        assert_eq!(a.display_name, Some("bob smith".to_string()));
        assert_eq!(name_of(&manager, a.id), "bob smith");
    }

    #[test]
    fn renaming_to_a_padded_form_of_the_current_name_burns_no_number() {
        // ★개명 멱등 계약과의 접점★: 정규화가 결정표보다 앞에 있어야 `" bob "` 이 자기 현재 이름과 같은
        //   값으로 판정된다. 뒤에 있으면 `" bob "` 이 빈 이름으로 보여 **다른 이름**으로 확정되고, 그 순간
        //   `bob` 이 비어 다음 요청자가 화면상 동명을 그대로 가져간다.
        let manager = bare_manager();
        let bob = create(&manager, "C:/a", Some("bob"));

        assert_eq!(
            manager.rename_agent(bob.id, Some("  bob  ".into())),
            RenameOutcome::Renamed("bob".to_string()),
            "요청은 `bob` 요청이므로 이름이 바뀌지 않는다"
        );
        assert_eq!(name_of(&manager, bob.id), "bob");
        assert_eq!(
            manager.agent_snapshot(bob.id).unwrap().display_name,
            Some("bob".to_string())
        );
        // 번호도 태우지 않았다 — 다음 신규가 bob(1) 이다(bob 이 여전히 점유돼 있다는 뜻이기도 하다).
        let next = create(&manager, "C:/b", Some("bob"));
        assert_eq!(
            next.canonical_name_when_live(),
            "bob(1)",
            "패딩 개명이 bob 을 비웠으면 여기서 동명 두 건이 앉는다"
        );
    }

    #[test]
    fn a_padded_request_for_a_taken_name_gets_the_suffixed_form() {
        // 남이 `bob` 을 쥔 상태의 `" bob "` 요청은 `bob` 요청과 **같은 취급**이어야 한다 — 별개 이름으로
        //   저장되면 유일성 검사를 우회한 동명 2건이 된다.
        let manager = bare_manager();
        create(&manager, "C:/h", Some("bob"));

        let other = create(&manager, "C:/o", Some("  bob  "));
        assert_eq!(
            other.display_name,
            Some("bob(1)".to_string()),
            "패딩 요청도 접미사 계열로 들어간다"
        );
        assert_eq!(name_of(&manager, other.id), "bob(1)");

        // 이미 그 계열 이름을 쥔 뒤의 패딩 재요청 = 멱등 무변경(번호 미소모).
        assert_eq!(
            manager.rename_agent(other.id, Some(" bob ".into())),
            RenameOutcome::Unchanged("bob(1)".to_string())
        );
        let third = create(&manager, "C:/t", Some("bob"));
        assert_eq!(
            third.canonical_name_when_live(),
            "bob(2)",
            "재요청이 번호를 태웠으면 여기가 bob(3) 이 된다"
        );
    }

    /// `capabilities()` 가 처음 불릴 때 콜백을 1회 실행하는 테스트 transport.
    ///
    /// ★왜 그 지점인가★: `roster()` → `list_agents()` → `agent_info()` 가 세션마다 `capabilities()` 를
    ///   부른다. 즉 `rename_agent` 의 **관측 도중** 임의 코드를 끼울 수 있는 유일한 주입점이라, "커밋 직전에
    ///   프로필이 사라지는" 창을 스레드·타이밍 없이 결정적으로 재현할 수 있다.
    struct HookedTransport {
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }
    impl AgentTransport for HookedTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            if let Some(h) = self.hook.lock().expect("hook poisoned").take() {
                h();
            }
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    /// 지정 transport 로 산 세션을 맵에 꽂는다(`put_live_session_at` 의 주입 가능 변형).
    fn put_live_session_with(
        manager: &AgentManager,
        id: AgentId,
        cwd: &str,
        transport: Box<dyn AgentTransport>,
    ) {
        let core = Arc::new(OutputCore::new(id, 0, Arc::new(NoopStatus)));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(cwd),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            InputEncoder::Raw,
            core,
            transport,
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
    }

    #[test]
    fn a_delete_landing_mid_rename_reports_not_found_not_success() {
        // ★배정 게이트는 **이름 배정끼리만** 직렬화한다 — 삭제는 이 게이트를 잡지 않는다★. 그래서
        //   `get` → `roster()` → `rename` 이 각각 프로필 락을 잡는 사이 다른 연결의 `DeleteProfile` 이 끼면
        //   커밋이 대상을 못 찾는다. 그 false 를 삼키고 성공으로 보고하면 wire 가 **없는 에이전트**에 Ack 를
        //   내고 `ProfileListUpdated` 까지 방송한다(사용자·LLM 이 지워진 것을 개명됐다고 본다).
        //   창은 좁지도 않다: `roster()` 는 override 없는 잠든 에이전트 1건당 canonicalize syscall 을 치른다.
        let manager = bare_manager();
        let victim = create(&manager, "C:/victim", Some("before"));
        let vid = victim.id;

        // 관측 도중(= 커밋 직전) 프로필이 사라지게 만든다.
        let profiles = Arc::clone(&manager.profiles);
        put_live_session_with(
            &manager,
            vid,
            "C:/live/victim",
            Box::new(HookedTransport {
                hook: Mutex::new(Some(Box::new(move || profiles.remove(vid)))),
            }),
        );

        // 결정표 3(Free) 갈래 — 요청 이름이 비어 있어 접미사 없이 커밋하는 경로.
        let out = manager.rename_agent(vid, Some("after".into()));
        assert_eq!(
            out,
            RenameOutcome::NotFound,
            "착지하지 않은 커밋을 성공으로 보고하면 안 된다(Free 갈래)"
        );
        assert!(
            manager.agent_snapshot(vid).is_none(),
            "전제 — 프로필은 실제로 사라졌다"
        );

        // ★결정표 4b(Suffixed) 갈래도 같은 커밋을 한다 — 두 arm 을 따로 봉인한다★.
        let holder = create(&manager, "C:/holder", Some("taken"));
        let victim2 = create(&manager, "C:/victim2", Some("other"));
        let vid2 = victim2.id;
        let profiles2 = Arc::clone(&manager.profiles);
        put_live_session_with(
            &manager,
            vid2,
            "C:/live/victim2",
            Box::new(HookedTransport {
                hook: Mutex::new(Some(Box::new(move || profiles2.remove(vid2)))),
            }),
        );
        let out2 = manager.rename_agent(vid2, Some("taken".into()));
        assert_eq!(
            out2,
            RenameOutcome::NotFound,
            "접미사 배정 갈래의 커밋 결과도 삼키면 안 된다"
        );
        assert!(manager.agent_snapshot(vid2).is_none());
        assert_eq!(
            name_of(&manager, holder.id),
            "taken",
            "실패한 개명이 남의 이름을 건드리지 않는다"
        );
    }

    #[test]
    fn clearing_an_override_predicts_the_name_from_the_live_axis() {
        // ★해제 후 이름을 낳는 축은 `session.cwd` 다★(명부가 산 에이전트에 그 축을 쓴다). `profile.cwd` 로
        //   예측하면 두 축이 갈리는 순간 **남이 쥔 이름을 Free 로 오판**해 커밋하고, 보고하는 이름도 실제와
        //   다르다 — ADR-0120 전역 유일성이 깨진다. 두 축은 spawn 시 같은 canonicalize 로 만들어지므로
        //   갈리려면 그 뒤의 파일시스템 변화(junction 재지정·디렉터리 개명·cwd 삭제)가 필요하다.
        let manager = bare_manager();
        // Y 가 산 축의 basename("q")을 이미 쥔다.
        let y = create(&manager, "C:/other", Some("q"));
        // X: override "x", 프로필 축 basename "p", 산 축 basename "q"(= Y 와 충돌하는 쪽).
        let x = create(&manager, "C:/prof/p", Some("x"));
        put_live_session_at(&manager, x.id, "C:/live/q");
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "전제 — override 가 이름"
        );

        // override 해제 → 실제로는 "q" 가 되려 하고, 그건 Y 가 쥐고 있다 → 접미사가 붙어야 한다.
        let out = manager.rename_agent(x.id, None);
        assert_eq!(
            out,
            RenameOutcome::Renamed("q(1)".to_string()),
            "프로필 축('p')으로 예측하면 Free 로 오판해 Renamed(\"p\") 를 보고한다"
        );
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "q(1)",
            "명부에서도 접미사 이름이어야 한다"
        );
        assert_eq!(name_of_in_roster(&manager, y.id), "q", "Y 의 이름은 그대로");
        // 어떤 이름도 중복되지 않는다(이 결함의 실제 피해).
        let roster = manager.roster();
        let unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            unique.len(),
            roster.len(),
            "명부에 동명이 앉으면 안 된다: {roster:?}"
        );
    }

    #[test]
    fn clearing_an_override_predicts_with_the_live_derivation_not_the_dormant_one() {
        // ★두 축은 재료뿐 아니라 **함수**도 다르다★: 산 축은 raw `session.cwd` basename(canonicalize 없음),
        //   잠든 축은 canonicalize 후 basename. 산 에이전트의 예측을 잠든 함수에 태우면
        //   `basename(canonicalize(cwd)) == basename(cwd)` 라는 가정에 기대게 되고, 그 가정이 깨지면
        //   **남이 쥔 이름을 Free 로 오판해 커밋**한다(ADR-0120 전역 유일성 붕괴).
        // 이 픽스처는 그 갈림을 `<temp>/.` 로 직접 만든다 — raw basename 은 `"."`, canonicalize 하면 `<temp>`
        //   의 마지막 세그먼트다.
        let temp = std::env::temp_dir();
        let dotted = format!("{}/.", temp.to_string_lossy());
        let raw_base = crate::agent::name::cwd_basename(&dotted);
        let canon_base = {
            let c = dunce::canonicalize(&dotted).expect("temp 디렉터리는 실재한다");
            crate::agent::name::cwd_basename(&c.to_string_lossy())
        };
        assert_ne!(
            raw_base, canon_base,
            "이 테스트의 전제 — 두 축이 실제로 갈려야 한다"
        );

        let manager = bare_manager();
        // Y 가 **raw 축** 이름을 쥔다(= 해제 후 X 가 실제로 갖게 될 이름).
        let y = create(&manager, "C:/other", Some(&raw_base));
        // X: override "x", 산 세션 cwd = 갈림을 만드는 그 경로.
        let x = create(&manager, "C:/prof/whatever", Some("x"));
        put_live_session_at(&manager, x.id, &dotted);
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "전제 — override 가 이름"
        );

        // 해제 → 실제 결과는 raw 축 이름이고 그건 Y 가 쥐었다 → 접미사가 붙어야 한다.
        let out = manager.rename_agent(x.id, None);
        assert_eq!(
            out,
            RenameOutcome::Renamed(format!("{raw_base}(1)")),
            "잠든 함수로 예측하면 canonicalize 된 이름(비어 있음)을 보고 Free 로 오판한다"
        );
        assert_eq!(name_of_in_roster(&manager, x.id), format!("{raw_base}(1)"));
        assert_eq!(name_of_in_roster(&manager, y.id), raw_base, "Y 는 그대로");
        let roster = manager.roster();
        let unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            unique.len(),
            roster.len(),
            "명부에 동명이 앉으면 안 된다: {roster:?}"
        );
    }

    #[test]
    fn concurrent_creates_of_one_name_never_both_take_it() {
        // ★배정 게이트의 존재 이유 그 자체★: 파생·관측·커밋이 한 임계구역이 아니면 동시 요청 둘 이상이
        //   같은 이름을 "비어 있다" 고 보고 **둘 다** 가져간다. 순차 테스트로는 절대 드러나지 않는다.
        // ★Barrier 로 겹침을 **설계로** 만든다★: 배리어 없이는 스레드들이 순차로 흘러 우연히 겹칠 때만
        //   경합이 재현된다(그 우연은 하네스의 디스크 쓰기 지연에 얹혀 있어 보증이 아니다). 전원이 배리어를
        //   통과한 직후 동시에 배정에 진입하므로 게이트가 없으면 여러 스레드가 같은 관측을 본다.
        // ★그래도 확률적 탐지다(정직 명시)★: 배리어는 **진입 시점**만 맞추고 그 뒤 인터리빙은 스케줄러
        //   소관이다. 게이트를 제거하면 거의 항상 실패하지만 운 좋게 통과할 수 있다. 결정적 봉인은 배정
        //   지점의 yield-seam 이 필요하고 그건 별도 결정이다.
        const THREADS: usize = 8;
        let manager = Arc::new(bare_manager());
        let start = Arc::new(std::sync::Barrier::new(THREADS));
        let names: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let m = Arc::clone(&manager);
                    let start = Arc::clone(&start);
                    scope.spawn(move || {
                        // 프로필 조립은 배리어 **전에** 끝내 배정만 동시에 시작하게 한다.
                        let p = agent_profile("C:/race", Some("bob"));
                        start.wait();
                        m.create_agent(p)
                            .expect("배정 성공")
                            .canonical_name_when_live()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let unique: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(
            unique.len(),
            THREADS,
            "동시 배정이 서로 다른 이름을 받아야 한다(중복 = 유일성 붕괴): {names:?}"
        );
        // 명부 쪽에서도 같은 사실이 보여야 한다(커밋까지 직렬화됐다는 뜻).
        let roster = manager.roster();
        assert_eq!(roster.len(), THREADS);
        let roster_unique: std::collections::BTreeSet<&String> =
            roster.iter().map(|e| &e.canonical_name).collect();
        assert_eq!(
            roster_unique.len(),
            THREADS,
            "명부에 동명이 앉으면 안 된다: {roster:?}"
        );
    }

    /// 실 spawn 경로가 정말 `register_for_spawn` 을 탄다는 것(= 위 단위 테스트가 죽은 코드를 보고 있지
    /// 않다는 것)을 실 프로세스로 확인한다. cmd.exe 두 개를 잠깐 띄웠다 죽인다.
    #[cfg(windows)]
    #[test]
    fn spawning_a_brand_new_agent_with_a_taken_name_gets_a_suffix() {
        let manager = bare_manager();
        let existing = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("bob"),
        );
        let first = manager
            .spawn_agent(&existing, SpawnMode::Fresh)
            .expect("기존 에이전트 spawn");
        assert_eq!(
            first.name, "bob",
            "이미 명부에 있는 에이전트를 띄우는 건 개명 대상이 아니다"
        );

        // ad-hoc spawn(연결이 그 자리에서 만든 프로필 = 명부에 없는 id) — 신규 등록이라 접미사.
        let adhoc = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("bob"));
        let second = manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect("ad-hoc spawn");
        assert_eq!(second.name, "bob(1)", "신규 등록 spawn 은 접미사를 받는다");

        manager.kill_agent(first.id).ok();
        manager.kill_agent(second.id).ok();
    }
}
