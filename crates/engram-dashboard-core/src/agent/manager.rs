//! AgentManager — backend/transport/output_core/session 을 묶어 에이전트 생명주기를 관리한다.
//!
//! tauri import 0 — 상위 상태 알림은 StatusSink trait으로 주입받는다(AppHandle 아님).
//!
//! ★명부(roster) 단일 소유자(ADR-0119)★: "전체 에이전트 + 각자 살아있음/잠듦" 은 `roster()` 한 곳에서만
//! 만들어진다. 프로필 레지스트리는 이 타입 **안**에 있고 밖으로 핸들이 나가지 않는다 — 바깥은 좁은
//! 동사(create/delete/rename/reparent/set-auto-restore/snapshot)만 쓴다.
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
use crate::agent::output_core::{OutputCore, TurnWiring};
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
use crate::agent::turn::TurnObservations;
use crate::agent::types::{
    AgentId, AgentInfo, AgentStatus, BackendCaps, CommandSpec, ControlChannel, NoopControlChannel,
    OutputChunk, OutputEvent, OutputSink, PtyError, ReapMsg, SinkId, StatusSink, SubscribeOutcome,
    TerminalReason, TerminationIntent,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// resume spawn 후 이 시간 안에 비정상 종료(code≠0/Failed/Killed)하면 resume 실패로 판정한다
/// (H-1.7 "조기 종료 윈도"). 성공한 resume은 TUI라 계속 떠 있다.
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);
/// 복원 시 에이전트 간 spawn 간격(동시 폭주 방지 stagger).
const RESTORE_STAGGER: Duration = Duration::from_millis(200);

#[cfg(windows)]
pub fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
pub fn default_shell() -> &'static str {
    "bash"
}

/// 별도 함수로 뺀 이유 = 실 claude 없이 선택 로직을 단위 테스트하기 위함
/// (ADR-0012 격리 — json→structured caps / 터미널→아님).
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
        // cols/rows 는 파이프에 개념이 없어 무시된다.
        let (t, pid) = StdioTransport::open(spec, true, decoder)?;
        Ok((Box::new(t), pid))
    } else {
        // decoder 는 여기서 버려진다 — `backend::output_decoder` 가 json 모드에만 Some 을 주므로
        // non-json 은 애초에 None 이 온다.
        let (t, pid) = PtyTransport::open(spec, cols, rows)?;
        Ok((Box::new(t), pid))
    }
}

/// 명부(roster) 항목 하나 = **에이전트 하나**(ADR-0119 결정 1). "산 목록"과 "프로필 목록"을 소비자가
/// 각자 합치던 중복을 없애는 것이 이 타입의 존재 이유다.
///
/// ★터미널 상태로 맵에 남은 시체는 항목이 아니다★ — 산 것도 잠든 것도 아니다. 프로필이 남아 있으면
///   잠듦으로, 없으면(ad-hoc) 아예 목록에 없다.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    pub id: AgentId,
    pub canonical_name: String,
    /// `Some` = 살아 있음(Running|Exiting 세션 부착) · `None` = 잠듦(프로필만 있음).
    pub live: Option<AgentInfo>,
    /// 작업 폴더. 산 항목은 세션 cwd(spawn 시 canonicalize), 잠든 항목은 저장된 raw cwd — **이름 파생이
    /// 보는 재료와 같은 것**이다(`roster()` 의 "산/잠듦 출처가 다르다" 규율). 세션도 프로필도 없으면 빈 문자열.
    pub cwd: String,
    /// 트리 부모. 저장 계층이 유일한 출처이므로 산 항목도 프로필에서 읽는다 — `None` = 최상위.
    pub parent: Option<AgentId>,
}

/// **지금 실제로 override 를 싣는 경로는 개명 하나뿐**이다 — 표시명 override 를 나르는 wire 명령은
/// `RenameProfile` 뿐이고, 생성·spawn 쪽에는 그 필드가 아예 없다. 그래서 `create_agent`·
/// `register_for_spawn` 쪽 호출은 공개 API 방어선이다.
///
/// ★저장된 이름 · 화면에 그려지는 이름 · 편지 주소가 **같은 문자열**이어야 한다★: 유일성(ADR-0120) 판정은
///   문자열 비교라 `bob` 과 `" bob "` 은 서로 다른 이름으로 **둘 다** 통과하는데 트리에는 똑같이 그려진다 —
///   사용자는 편지가 둘 중 누구에게 가는지 구분할 수 없다. 게다가 메시징 입구가 수신자 토큰을 trim 해서
///   맞추므로 `" bob "` 으로 저장된 에이전트는 보이면서도 이름으로 주소 지정이 안 된다.
/// ★그 입구 trim(`messaging` service 수신자 대조)은 지우지 말 것★: **CLI 입구**(`engram mail send --to a,b`)가
///   셸 제약 때문에 수신자 목록을 콤마로 쪼개는데 그때 공백을 떼지 않아(`"alice, bob"` → `["alice", " bob"]`)
///   두 번째 이후 수신자를 그 trim 이 구제한다. 저장을 정규화하면 그 trim 은 잘 저장된 이름에 대해
///   no-op 이 될 뿐이고, 콤마 목록 구제 역할은 그대로 남는다.
/// ★유일성 판정 **전에** 건다★: 판정과 저장이 같은 정규화 값을 봐야 `" bob "` 이 모든 면에서 `bob` 요청이
///   된다(뒤에 걸면 `" bob "` 이 빈 이름으로 판정돼 동명이 다시 새어 들어온다).
/// ★안쪽 공백은 이름의 일부다★ — `"bob smith"` 는 그대로 살아야 하므로 양끝만 깎는다.
/// ★남는 문제는 "같은 구멍의 잔여" 가 아니라 다른 종류다★: `str::trim` 은 Unicode White_Space(스페이스·탭·
///   NBSP)만 걷어내고 zero-width(U+200B 등)는 **양쪽 어디서도** 떨어지지 않는다 — 그래서 그런 이름은
///   저장 == 표시 == 주소가 그대로 성립해 위 불변식을 깨지 않는다(패딩 이름이 깬 것은 *주소 도달성*이었다).
///   남는 것은 눈으로 구분이 안 되는 **시각적 혼동**뿐이고, 그 해법(NFKC·confusable folding)은 정당한
///   이름까지 뭉개므로 여기서 즉흥 필터로 처리하지 않는다 — 정책 결정 사항이다.
/// ★이미 저장된 이름은 고치지 않는다★ — 지금부터의 쓰기에만 걸린다(마이그레이션 장치 없음).
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

/// ADR-0115 표기(`이름(N)`)의 파서 — 발급기(`decide_name_with_roster` 의 `format!("{base}({n})")`)와
/// **정확한 역함수**여야 한다. 표기를 바꾸면 둘 다 바꾼다.
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

/// ★명부 총량 상한 = **폭주 백스톱**이지 제품 한도가 아니다★ (사용자 결정 2026-08-11).
///
/// 제어 평면은 모든 에이전트에게 열려 있고(ADR-0132 결정 5) 그 결정은 유지된다 — 그러나 무언가가
/// "에이전트 만들기" 를 반복 호출하면 명부와 `agents.json` 이 **상한 없이** 자란다. 그 루프만 끊는다.
///
/// ★왜 코어인가(입구가 아니라)★: 상한을 입구에 두면 입구마다 사본이 생기고, 사본이 없는 입구는 그냥
///   통과한다 — 실제로 제어 라우트에만 두었더니 데스크톱(WS `CreateProfile`)과 ad-hoc spawn 이 그대로
///   빠져나갔다. 등록을 **커밋하는 자리**에 두면 입구 수와 무관하게 참이고, 이름 배정 게이트 안이라
///   **원자적**이다(여러 요청이 각자 511 을 관측하고 다 같이 등록하는 창이 없다).
/// ★왜 개수 검사인가(속도 제한이 아니라)★: 타이머·호출자별 상태·튜닝 노브가 하나도 없어야 이 방어가 스스로
///   고장 나지 않는다. 세는 것은 명부 총량 하나뿐이다.
/// ★왜 이 숫자인가 — **튜닝하지 말 것**★: 정당한 사용이 도달할 수 없는 자리에 있으면 그만이다. 실제 팀
///   트리는 수십 단위다. **"자원이 먼저 바닥나니 상한은 형식" 이라는 논리는 쓰지 말 것** — 그 논리는
///   프로세스를 띄우는 등록에만 통하고, 프로세스를 하나도 띄우지 않는 **잠든 에이전트 등록**엔 통하지
///   않는다(그쪽은 아무것도 밀어내지 않으므로 물리적 제동이 아예 없다). 상한이 필요한 이유가 바로 그것이다.
/// ★기존 명부는 인질이 아니다★: 상한은 **신규 등록**만 본다 — 이미 상한을 넘은 명부의 복원·재spawn 은
///   그대로 돌아간다(`register_for_spawn` 의 기존-id 분기는 이 검사를 지나지 않는다).
// ADR-0132
pub const MAX_ROSTER_SIZE: usize = 512;

/// 신규 등록 전 총량 검사 — **이름 배정 게이트를 보유한 상태에서만** 부른다(그래야 원자적이다).
///
/// ★삭제와는 원자적이지 않다(의도)★: 삭제는 이 게이트를 잡지 않지만 개수를 **줄이기만** 하므로, 최악이
/// "방금 자리가 났는데 이번 호출은 거부" 이고 다음 호출이 통과한다. 백스톱에 필요한 방향의 안전이다.
fn check_roster_capacity(roster: &[RosterEntry]) -> Result<(), PtyError> {
    if roster.len() >= MAX_ROSTER_SIZE {
        return Err(PtyError::RosterFull {
            current: roster.len(),
            limit: MAX_ROSTER_SIZE,
        });
    }
    Ok(())
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

/// `used` = 그 계열이 지금 점유한 번호 집합.
///
/// 기본 규칙은 ADR-0115 그대로 **관측 최대 + 1**(같은 명부 안에서 단조 증가 — 산 것끼리 번호가 겹치지
/// 않는다). 별도 최고수위 상태는 없으므로 계열이 비면 번호는 1부터 다시 나온다(ADR-0123).
///
/// ★포화 탈출구(saturation-only)★: 최대가 `u32::MAX` 면 "최대 + 1" 이 없다. 그때만 **가장 낮은 빈
/// 번호**로 내려간다 — 안 그러면 `이름(4294967295)` 하나가 그 계열의 42억 개 빈 번호를 영구히 봉쇄한다.
/// 정상 경로는 단조 증가를 유지하고, 이 갈래는 최대가 MAX 일 때만 열린다.
/// `None` = 1..=MAX 가 전부 점유(현실 도달 불가 — 전역 함수로 두기 위한 종결 갈래).
// ADR-0123
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
    status_sink: Arc<dyn StatusSink>,
    // 프로필 단일 소유자.
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
    // ADR-0006
    name_allocation: Arc<Mutex<()>>,

    /// 턴 관측 표(ADR-0113 사실 계층) — 이 매니저가 띄운 모든 `OutputCore` 가 공유한다.
    /// 쓰기 = `OutputCore::emit`(★호출 스레드는 둘이다★ — 출력 pump 와 입력 에코를 낸 주입 스레드,
    /// turn.rs 헤더), 청소 = `OutputCore::finish`(★reaper 가 아니다★ — 종료 후 지각 emit 과의 경쟁을
    /// finalize 플래그와 같은 지점에서 닫아야 해서다. ADR-0127 결정 5 · 거부한 대안 (d)),
    /// 읽기 = 소비자(우편 idle 게이트 등).
    /// ★sessions 락과 무관한 leaf★: 이 표를 잡은 채 sessions/profiles 를 잡는 경로가 없다
    /// (ADR-0006 순서에 순환을 만들지 않는다).
    // ADR-0113
    // ADR-0127
    turns: Arc<TurnObservations>,
}

/// spawn 진행 중 AgentId 예약을 잡고, drop 시 자동 해제하는 RAII 가드(ADR-0086 FIX 6). spawn_agent
/// 의 어느 조기 반환(provision 실패·PTY 실패·`?`)에서도 예약이 새지 않게 한다. `reserve` 가 이미 예약된
/// id 면 None(두 번째 동시 spawn 거부).
struct SpawnReservation {
    spawning: Arc<Mutex<HashSet<AgentId>>>,
    id: AgentId,
}

impl SpawnReservation {
    fn reserve(spawning: Arc<Mutex<HashSet<AgentId>>>, id: AgentId) -> Option<Self> {
        {
            let mut set = spawning.lock().expect("spawning set poisoned");
            if !set.insert(id) {
                return None;
            }
        }
        Some(Self { spawning, id })
    }
}

impl Drop for SpawnReservation {
    fn drop(&mut self) {
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
    /// disarm 이후 등록된 세션의 revoke 는 kill_agent/reaper 소관이다(이중 revoke 방지).
    armed: bool,
}

impl ProvisionGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProvisionGuard {
    fn drop(&mut self) {
        if self.armed {
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
        let turns = Arc::new(TurnObservations::new());

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
            turns,
        }
    }

    pub fn turns(&self) -> Arc<TurnObservations> {
        self.turns.clone()
    }

    pub fn presets(&self) -> &Arc<PresetRegistry> {
        &self.presets
    }

    // ★우편 자격 조회 동사를 여기 두지 않는다(되돌리지 마라)★: 소비자는 명단 스냅샷의
    //   `AgentInfo::reads_messages` 를 읽는다. id 로 되묻는 동사를 만들면 그 자리에서 TOCTOU·비원자성·
    //   세션당 락이 되살아난다(그 필드 doc). 판정 출처는 세션이 spawn 때 backend 에서 받아 든 값이고
    //   (`AgentSession::reads_messages`), **프로필이 아니다** — `DeleteProfile` 은 산 세션을 죽이지 않아
    //   프로필 축은 운영에서 "모름" 이 되고, 그 fail-open 이 실제로 셸을 명단에 되돌린 구멍이었다.

    // ── 명부(roster) — 단일 입구 ────────────────────────────────────────────

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
        // ★프로필 스냅샷은 **한 장**이고, 산 항목의 이름·계층도 그 한 장에서 나온다(load-bearing)★.
        //   예전엔 산 항목의 이름을 `list_agents()` 가 세션마다 따로 뜬 프로필에서 얻고 계층은 뒤이은
        //   목록 조회에서 얻었다 — 그 사이에 개명 + 계층 이동이 커밋되면 **한 번도 존재한 적 없는 조합**
        //   (옛 이름 + 새 부모)이 한 행에 실린다. 한 장에서 뽑으면 그 조합 자체가 만들어질 수 없다.
        //   ★세션↔프로필 사이의 비원자성은 그대로 남고, 그건 이미 수용된 성질이다★(위 doc · ADR-0116).
        //   여기서 닫은 것은 프로필↔프로필 불일치다.
        // ★부수 효과 = 더 싸졌다★: 옛 경로는 세션 하나마다 `profiles.get`(= AgentProfile 통째 clone +
        //   뮤텍스 획득)을 했다. 이제 목록 1회로 끝나므로 산 세션 수만큼의 프로필 clone 이 사라진다 —
        //   이 조회는 우편 발송의 임계 경로에 있다(ADR-0119 · messaging_host `addressing_sources`).
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        let mut profiles: HashMap<AgentId, AgentProfile> = self
            .profiles
            .list()
            .into_iter()
            .map(|p| (p.id, p))
            .collect();
        let mut live_ids: HashSet<AgentId> = HashSet::with_capacity(sessions.len());
        let mut entries: Vec<RosterEntry> = Vec::with_capacity(sessions.len() + profiles.len());
        for session in &sessions {
            let info = self.agent_info_with(
                session,
                profiles
                    .get(&session.id)
                    .and_then(|p| p.display_name.as_deref()),
            );
            // 시체(terminal)는 reap 까지 맵에 남는다 — 존재가 아니라 상태로 가른다(ADR-0116 술어).
            //   ★프로필은 맵에 남겨 둔다★: 시체의 프로필은 아래 루프에서 **잠듦** 항목으로 올라와야 한다.
            if !info.status.is_live() {
                continue;
            }
            live_ids.insert(info.id);
            entries.push(RosterEntry {
                id: info.id,
                canonical_name: info.name.clone(),
                cwd: info.cwd.clone(),
                parent: profiles.get(&info.id).and_then(|p| p.parent_id),
                live: Some(info),
            });
        }
        for (_, p) in profiles.drain() {
            if live_ids.contains(&p.id) {
                continue;
            }
            entries.push(RosterEntry {
                id: p.id,
                canonical_name: p.canonical_name_when_live(),
                // 소유한 값이라 **옮긴다** — 유효 UTF-8 이면 버퍼 재사용이고, 아니면 그때만 lossy 사본.
                cwd: p
                    .cwd
                    .into_os_string()
                    .into_string()
                    .unwrap_or_else(|os| os.to_string_lossy().to_string()),
                parent: p.parent_id,
                live: None,
            });
        }
        entries
    }

    pub fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        self.profiles.get(id)
    }

    /// wire `ProfileList` 가 아직 프로필 타입을 그대로 나르므로 데이터는 경계를 넘지만, 레지스트리
    /// 핸들은 넘지 않는다.
    pub fn agent_snapshots(&self) -> Vec<AgentProfile> {
        self.profiles.list()
    }

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
    pub fn create_agent(&self, mut profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        profile.display_name = normalize_display_name(profile.display_name.take());
        // ★파생도 게이트 안에서★: 요청 이름을 정하는 읽기가 게이트 밖에 있으면 관측과 커밋 사이가 아니라
        //   **파생과 관측 사이**에 창이 생긴다(그 사이 남이 같은 이름을 커밋하면 둘 다 자유로 판정한다).
        let _gate = self.lock_name_allocation();
        // 명부 한 장으로 상한과 이름을 **함께** 판정한다 — 두 번 뜨면 그 사이가 다시 창이 되고, 조회 비용도
        //   두 배가 된다.
        let roster = self.roster();
        check_roster_capacity(&roster)?;
        let desired = profile.canonical_name_when_live();
        match self.decide_name_with_roster(&roster, profile.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => profile.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) 은 KeepCurrent 를 낼 수 없다")
            }
        }
        self.profiles.upsert(profile.clone());
        Ok(profile)
    }

    /// 에이전트 삭제(트리 "지우기").
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
    pub fn rename_agent(&self, id: AgentId, display_name: Option<String>) -> RenameOutcome {
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
            Some(live_cwd) => crate::agent::name::canonical_name_or_id_fallback(
                display_name.as_deref(),
                &live_cwd,
                id,
            ),
            None => {
                let mut probe = profile;
                probe.display_name = display_name.clone();
                probe.canonical_name_when_live()
            }
        };
        match self.decide_name_with_roster(&roster, id, &desired, Some(&current)) {
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
            NameDecision::KeepCurrent => RenameOutcome::Unchanged(current),
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
    /// 트리 계층 이동(부모 지정/해제).
    pub fn reparent_agent(&self, child_id: AgentId, parent_id: Option<AgentId>) -> bool {
        self.profiles.reparent(child_id, parent_id)
    }

    /// 부팅 자동 복원 대상 토글 — 없는 id 면 false.
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
    ///
    /// ★로스터 스냅샷을 **받는다**(스스로 뜨지 않는다)★: 호출자는 같은 게이트 구간에서 그 명부로 총량 상한도
    ///   보고(개명은 자기 현재 이름도 거기서 읽는다) — 두 번 뜨면 판정마다 다른 세계를 보게 되고 조회 비용도
    ///   배가 된다.
    // ADR-0120
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
    // ADR-0115
    fn register_for_spawn(&self, profile: &AgentProfile) -> Result<(), PtyError> {
        // 기존 id 면 이름 배정 자체가 없으므로 게이트를 잡지 않는다.
        if self.profiles.get(profile.id).is_some() {
            self.profiles.upsert_preserving_hierarchy(profile.clone());
            return Ok(());
        }
        let mut fresh = profile.clone();
        fresh.display_name = normalize_display_name(fresh.display_name.take());
        let _gate = self.lock_name_allocation();
        // ★상한은 여기도 본다★: 이 분기가 **ad-hoc spawn 의 신규 등록 지점**이라, 여기를 비워 두면 명부가
        //   `create_agent` 를 거치지 않고도 무한히 자란다(상한이 "총량" 이라는 말이 거짓이 된다).
        let roster = self.roster();
        check_roster_capacity(&roster)?;
        let desired = fresh.canonical_name_when_live();
        match self.decide_name_with_roster(&roster, fresh.id, &desired, None) {
            NameDecision::Free => {}
            NameDecision::Suffixed(assigned) => fresh.display_name = Some(assigned),
            NameDecision::Exhausted => return Err(name_space_exhausted(&desired)),
            NameDecision::KeepCurrent => {
                unreachable!("decide_name(current=None) 은 KeepCurrent 를 낼 수 없다")
            }
        }
        self.profiles.upsert_preserving_hierarchy(fresh);
        Ok(())
    }

    // ── spawn ──────────────────────────────────────────────────────────────

    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // ★잔여 레이스(ADR-0082 미해결·후속)★: 이 이중-spawn 가드는 여기서 read lock 을 잡아 contains_key 를 본 뒤
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

        let _reservation = SpawnReservation::reserve(self.spawning.clone(), profile.id)
            .ok_or_else(|| {
                PtyError::SpawnFailed(format!(
                    "agent {} spawn already in progress (concurrent spawn rejected)",
                    profile.id
                ))
            })?;

        self.register_for_spawn(profile)?;

        // cwd 정규화 — claude 세션 디렉토리 표기 고정(UNC 회피). 실패 시 원본 사용(best-effort).
        let cwd = dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone());

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

        // ★화신 표식 확정 — 화신마다 새 값(ADR-0007)★
        //
        // ★왜 이 자리인가(호출부가 아니라)★: 맵 교체가 실제로 일어나는 곳이 여기고, **모든 spawn 이
        //   모드와 무관하게 이 한 줄을 지난다**. 옛날엔 발급이 `activate_profile` 의 Resume 갈래에만
        //   있어서 Fresh 재spawn 경로들(WS `Spawn` 명령 · `activate_profile` 의 Fresh 갈래 · sid 없는
        //   프로필의 부팅 복원)이 죽은 화신의 표식을 **그대로 재사용**했다. 그 재사용은 (AgentId, epoch)
        //   를 키로 쓰는 모든 구조를 무너뜨린다 — 턴 관측 표(ADR-0113: 죽은 화신의 지각 emit 이 산 화신의
        //   항목을 덮고 그 emit 의 finalize 재확인이 그걸 지운다) · 제어 채널 토큰(ADR-0086) ·
        //   reap epoch-guard(ADR-0084). 호출부마다 흩뿌리면 새 호출부가 또 빠뜨린다.
        // ★모드를 보지 않는다(Resume 도 같은 규칙)★: 새 프로세스를 띄웠으면 그건 새 화신이고, 모드는 그
        //   사실을 바꾸지 않는다. 모드로 가르면 Resume 쪽 재spawn(`restore_all` 을 부팅 밖에서 부르는 등)이
        //   같은 재사용 구멍으로 남는다 — 그건 규약일 뿐 강제되지 않는다.
        // ★프로필이 사라졌으면 **spawn 을 중단한다**★: 삭제된 프로필로 세션을 띄울 이유가 없다. `?` 로 끊는다.
        // ADR-0007
        let epoch = self.profiles.epoch_for_spawn(profile.id).ok_or_else(|| {
            PtyError::SpawnFailed(format!(
                "profile {} vanished mid-spawn (concurrent delete) — spawn aborted",
                profile.id
            ))
        })?;

        // ADR-0086 ★spec 조립 직전에 부른다★ — build_command_spec 이 endpoint 를 받아 backend 방식
        //   (claude=`--mcp-config`)으로 명령줄에 주입해야 하므로. 화신 표식은 위에서 확정된 현재값이라
        //   화신이 바뀔 때마다 새 토큰이 발급된다(토큰 수명=(AgentId,epoch)).
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
        let control_endpoint = if backend::supports_control_channel(&profile.command) {
            // ADR-0099: backend 의 MCP-capability 를 provision 에 넘겨 채널 물리 배선·프라이밍 변형·grant 를
            //   한꺼번에 가르게 한다(정합 불변식 = 가르치는 채널 ⊆ 깐 채널 — ADR-0126 결정 4 로 단방향 개정).
            //   판정은 backend dispatch(ADR-0004) — manager 는 command 를 직접 matches! 하지 않는다.
            // ADR-0126
            let accepts_mcp = backend::accepts_mcp_config(&profile.command);
            self.control
                .provision(profile.id, epoch, accepts_mcp)
                .map_err(|e| {
                    PtyError::SpawnFailed(format!(
                        "control channel provision failed (fail-closed): {e}"
                    ))
                })?
        } else {
            None
        };
        let mut provision_guard = control_endpoint.as_ref().map(|_| ProvisionGuard {
            control: self.control.clone(),
            id: profile.id,
            epoch,
            armed: true,
        });

        let spec = backend::build_command_spec(
            &profile.command,
            mode,
            sid,
            cwd.clone(),
            profile.env.clone(),
            control_endpoint,
        );

        // spec 은 backend-neutral(program/args뿐)이라 caps 를 spec 에 싣지 않고 따로 전달한다 —
        // session 이 transport caps 와 compose 한다.
        let bcaps = backend::backend_caps(&profile.command);

        // ADR-0044: 판정은 프로필 command 단일 출처 — spawn_session 은 backend 를 모르므로
        // encoder/decoder/turn_classifier 를 여기서 뽑아 넘긴다.
        let json_mode = profile.command.is_json_mode();
        let encoder = backend::input_encoder(&profile.command);
        let decoder = backend::output_decoder(&profile.command);
        let turn_classifier = backend::turn_classifier(&profile.command);
        // 우편 자격도 여기서 뽑아 세션에 싣는다 — 프로필이 지워져도 산 세션이 그 사실을 계속 안다
        //   (`AgentManager::reads_messages` doc).
        let reads_messages = backend::reads_messages(&profile.command);

        // ADR-0079: json 모드 claude 만 실제로 transcript 를 읽는다 — 터미널은 TUI PTY repaint 로
        //   복원되고 shell 은 대화가 없어, 그 외 backend 는 빈 Vec 을 돌려준다.
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
            reads_messages,
            decoder,
            json_mode,
            epoch,
            seed_events,
            turn_classifier,
        )?;

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
    ///    오인 → `fallback_fresh` 가 **멀쩡히 돌던 산 에이전트를 kill** → 새 화신 표식 → 빈 fresh 로 교체
    ///    (유저 실측 회귀). 이제 가드 Err 에 닿기 전에 선제 contains_key 로 걸러 산 에이전트를 놔둔다.
    /// 2. **Fresh(진짜 신규 — 세션 없음)** — `spawn_agent(Fresh)` 위임. 이건 실패-fallback 이 아니라
    ///    정상 신규 생성이다(ADR-0076 "Fresh=새 sid" 유효).
    /// 3. **Resume** — `resume_no_fallback` 로 이어받기만 시도하고, 그 Failed 결말을 Err 로 노출한다.
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
        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: 이미 실행 중 — 재활성화 무시(산 에이전트 보존, ADR-0082)"
            );
            return Ok(self.agent_info(&session));
        }

        if mode == SpawnMode::Fresh {
            return self.spawn_agent(profile, SpawnMode::Fresh);
        }

        match self.resume_no_fallback(profile) {
            RestoreOutcome::Resumed => self.agent_info_by_id(profile.id),
            RestoreOutcome::Failed { reason } => Err(PtyError::SpawnFailed(reason)),
            // resumable 프로필로만 진입하므로 Started/Blocked/FreshFallback 은 도달 불가(방어적 Err).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: 예상 밖 결말 {other:?}"
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        reads_messages: bool,
        decoder: Option<Box<dyn OutputDecoder>>,
        json_mode: bool,
        epoch: u32,
        seed_events: Vec<OutputEvent>,
        turn_classifier: backend::TurnClassifier,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        let (transport, child_pid) =
            select_transport(json_mode, &spec, DEFAULT_COLS, DEFAULT_ROWS, decoder)?;

        // ADR-0113: 공용 턴 관측 표 + 이 백엔드의 신호 분류자를 함께 꽂는다 — 안 꽂으면 이 세션만
        //   조용히 관측 밖으로 빠진다.
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            self.status_sink.clone(),
            TurnWiring::new(self.turns.clone(), turn_classifier),
        ));

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

        // ★ADR-0019 finish-snapshot hook★: core.finish 의 finalize 승자 경로에서 1회 호출되며,
        //   **그 순간** intent·shutting_down 을 snapshot 해 ReapMsg 를 송신한다(reap 시점 live read
        //   금지 — 크래시→유저kill 오분류 race 방지). send 실패(reaper 종료)는 무시.
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
                    intent_at_finish: TerminationIntent::from_u8(
                        intent_hook.load(Ordering::SeqCst),
                    ),
                    shutting_down_at_finish: shutting_down_hook.load(Ordering::SeqCst),
                };
                let _ = reaper_tx.send(ReaperCmd::Reap(msg));
            }));
        }

        let session = Arc::new(AgentSession::new(
            id,
            spec.cwd.clone(),
            epoch,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            intent,
            backend_caps,
            encoder,
            reads_messages,
            core,
            transport,
        ));

        // ★ADR-0113 턴 관측 자리 선점 — sessions 맵 insert 보다 **먼저**★: 이 화신이 그 id 의 항목을
        //   차지한다(앞 화신의 항목이 있으면 갈아치운다). insert 전이라 아직 아무 스레드도 이 core 에
        //   닿을 수 없고(구독·emit 경로가 전부 sessions 조회를 거친다), 그래서 이 화신의 **첫 신호보다
        //   반드시 먼저**다. 뒤집히면 그 첫 신호가 앞 화신 표식과 안 맞아 버려지고, 이 화신은 앞 화신의
        //   항목이 거둬질 때까지 미관측(=턴 아님)으로 답한다 — 턴 중 우편 주입이 그 결말이다.
        //   ★표식 대소로 거르던 옛 규칙을 여기로 옮긴 것이다★(난수 표식에선 대소가 절반만 맞는다 —
        //   `turn::TurnObservations::register`).
        // ADR-0113
        self.turns.register(id, epoch);

        // ★ADR-0019 — sessions 등록은 pump 기동(start)보다 **먼저**★: finish hook 이 ReapMsg 를 보내는데,
        //    pump 가 즉시 EOF→finish 하면 그 시점에 세션이 맵에 있어야 reaper 가 reap 한다. insert 전에
        //    start 하면 빠른 종료 시 hook send 가 맵에 없는 id 를 가리켜 reap 가 no-op→세션 좀비화.
        //    attach_pump 는 start 내부 동기 완료라 join_pump 영향 없음(insert 순서 무관).
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

        session.start_pump();

        Ok((session, child_pid))
    }

    // ── 복원 ────────────────────────────────────────────────────────────────

    /// **백그라운드 스레드에서 호출할 것**(stagger·조기종료 윈도 대기로 블로킹 — setup 동기 호출
    /// 금지, H-1.8).
    pub fn restore_all(&self) -> Vec<RestoreReport> {
        let targets = self.profiles.restorable();
        tracing::info!(count = targets.len(), "restore_all 시작");

        let mut reports = Vec::with_capacity(targets.len());
        for profile in targets {
            let outcome = self.restore_one(&profile);
            // spawn 이 성공했으면 `epoch_for_spawn` 이 발급한 최신 표식이 명부에 있으므로 그걸 읽는다.
            //   프로필이 없으면 spawn 이 실패한 경우뿐이라 결말이 Failed 이고, 이때 스냅샷 값은
            //   보고용 표기일 뿐이다.
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

    fn restore_one(&self, profile: &AgentProfile) -> RestoreOutcome {
        let resumable =
            backend::needs_session(&profile.command) && profile.claude_session_id.is_some();

        if !resumable {
            return match self.spawn_agent(profile, SpawnMode::Fresh) {
                Ok(_) => RestoreOutcome::Started,
                Err(e) => RestoreOutcome::Failed {
                    reason: e.to_string(),
                },
            };
        }

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
    ///   화신 표식 재발급·respawn 을 전부 걷어냈다 — ADR-0082 사용자 결정: "아무것도 죽지마, 새로 만들지마").
    /// 이 로직을 restore_one(부팅 복원)과 activate_profile(수동 활성화)이 **똑같이** 재사용한다.
    // ADR-0082
    fn resume_no_fallback(&self, profile: &AgentProfile) -> RestoreOutcome {
        match self.spawn_agent(profile, SpawnMode::Resume) {
            Err(e) => {
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
            Ok(_) => match self.early_terminal_status(profile.id, EARLY_EXIT_WINDOW) {
                Some(status) => {
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

    // ── 구독/입출력 ────────────────────────────────────────────────────────

    pub fn subscribe(
        &self,
        agent_id: AgentId,
        sink: Arc<dyn OutputSink>,
    ) -> Result<SinkId, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.subscribe(sink))
    }

    /// `epoch_matches` 는 데몬이 요청 epoch 과 세션 현재 epoch 을 비교해 넘긴다 — 코어는 protocol
    /// 무의존이라 epoch 비교를 외부에서 받는다.
    ///
    /// ## ★계약: `Err` ⟹ `on_ready` 는 한 번도 불리지 않는다(load-bearing — 깨면 출력이 죽는다)★
    /// 실패는 **세션 조회 하나뿐**이고 그건 구조적으로 `on_ready` 를 넘기기 *전*이다. 이 순서에 데몬의
    /// 구독 거절 통보가 얹혀 있다: 데몬은 `Err` 를 `AgentEvent::SubscribeFailed` 로 바꿔 보내면서
    /// "이 구독엔 `SubscribeAck`(=`on_ready`)도 `ReplayComplete` 도 뒤따르지 않는다"를 계약으로 광고하고,
    /// 클라이언트(src-tauri)는 그 광고에 기대어 자기 single-flight 슬롯을 **즉시** 푼다.
    ///
    /// 그래서 이 함수에 `on_ready` 를 부른 *뒤* 실패할 수 있는 갈래를 추가하면, 거절과 Ack 가 같은 구독에
    /// 대해 함께 나가고 클라이언트가 이미 푼 슬롯 위로 늦은 Ack/Complete 가 도착해 **replay 가 돌지 않은
    /// 세대에 성공 마커**가 붙는다(gen 펜스 붕괴). 새 실패 갈래가 필요하면 `on_ready` 앞에 두거나, 거절
    /// 통보의 계약을 함께 고쳐야 한다. 회귀망 = `subscribe_from_err_never_invokes_on_ready`.
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

    pub fn unsubscribe(&self, agent_id: AgentId, sink_id: SinkId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        session.unsubscribe(sink_id);
        Ok(())
    }

    pub fn write_stdin(&self, agent_id: AgentId, data: &[u8]) -> Result<(), PtyError> {
        self.get_session(agent_id)?.write_input(data)
    }

    pub fn write_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::agent::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.write_input_observed(data)
    }

    /// `write_stdin_observed` 의 **제출 포함** 판 — 백엔드가 제출 바이트를 요구하면(터미널) 본문 뒤에
    /// 그것이 별도 write 로 한 번 더 나간다(근거·실측 = `AgentSession::submit_input_observed`).
    ///
    /// ★어느 쪽을 부를지 = 호출자의 성격★: "완성된 메시지 하나 = 턴 하나"(우편 배달)면 이것, 사람이
    ///   Enter 를 직접 치는 키 입력 스트리밍이면 `write_stdin`/`write_stdin_observed`.
    pub fn submit_stdin_observed(
        &self,
        agent_id: AgentId,
        data: &[u8],
    ) -> Result<crate::agent::types::WriteOutcome, PtyError> {
        self.get_session(agent_id)?.submit_input_observed(data)
    }

    /// ★incarnation 조건부 write★ — `expected_epoch` 가 **지금** 그 AgentId 가 가리키는 세션의 epoch 과
    ///   같을 때만 쓴다. 다르면 transport 를 아예 건드리지 않고(부작용 0) `Err` 를 낸다.
    ///
    /// ★왜 필요한가(check-then-write TOCTOU — load-bearing)★: 호출자가 `(id, epoch)` 로 수신자를 정한 뒤
    ///   `write_stdin_observed(id, ..)` 를 부르면, 그 사이 에이전트가 재시작(= 세션 맵 교체 + 새 화신 표식)했을 때
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
    // ADR-0088
    // ADR-0111
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

    /// ★하네스 전용★ — `insert_test_session` 의 짝. 이 매니저의 **통지 경로와 턴 관측 표에 이어진**
    ///   `OutputCore` 를 만든다(spawn_session 이 하는 배선과 동형).
    ///
    /// ★왜 필요한가★: 주입 세션이 `OutputCore::new` 만으로 조립되면 그 세션의 emit 은 매니저의 표에
    ///   닿지 않아, 게이트·도어벨 배선을 보려는 통합 테스트가 "관측이 없어서 통과" 하는 위약이 된다.
    ///   반대로 관측이 필요 없는 테스트는 이걸 쓰지 않으면 된다(운영 세션과 달리 선택이다).
    #[cfg(feature = "test-harness")]
    #[doc(hidden)]
    pub fn wired_test_core(
        &self,
        id: AgentId,
        epoch: u32,
        classify: crate::agent::backend::TurnClassifier,
    ) -> Arc<OutputCore> {
        Arc::new(OutputCore::new(
            id,
            epoch,
            self.status_sink.clone(),
            TurnWiring::new(self.turns.clone(), classify),
        ))
    }

    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.get_session(agent_id)?.resize(cols, rows)
    }

    pub fn interrupt(&self, agent_id: AgentId) -> Result<(), PtyError> {
        self.get_session(agent_id)?.interrupt()
    }

    // ── kill (LLD §6 절대순서) ───────────────────────────────────────────────

    /// 에이전트 종료(ADR-0019 reaper 위임). **맵 제거·disposition·통지는 하지 않는다** — pump 가 보낸
    /// ReapMsg 를 reaper 가 단일 소비해 처리한다. 그래서 반환 직후에도 세션이 아직 맵에 있을 수 있다 —
    /// 호출자가 "사라짐"을 단언하려면 폴링해야 한다(headless 테스트가 그렇게 한다).
    pub fn kill_agent(&self, agent_id: AgentId) -> Result<(), PtyError> {
        let session = self.get_session(agent_id)?;
        let epoch = session.epoch;

        // 0. ★제어 채널 토큰 즉시 폐기 — 블로킹 kill **전에**(FIX 4)★. 이 revoke 가 session.kill(최대
        //    5s join) **뒤**에 있으면 죽어가는 에이전트의 토큰이 그 5s 창 동안 유효해, 그 사이 에이전트가
        //    제어 채널로 명령을 낼 수 있다(TOCTOU). 여기선 락 미보유라 안전하고(§10 — registry 는 leaf
        //    lock, ADR-0006), revoke 는 idempotent 라 reaper 의 terminal revoke 와 겹쳐도 무해(그게 backstop).
        // ADR-0086
        self.control.revoke(agent_id, epoch);

        session.set_intent(TerminationIntent::UserKill);

        let _ = session.enter_exiting();

        // 1~6. ★revoke 배치가 이 인과를 건드리지 않는다(ADR-0001)★ — revoke 는 registry/파일만 만지고
        //       shutdown 체인에 개입하지 않아 kill 을 블록·재정렬하지 않는다. join 이 timeout 나도 그냥
        //       진행한다(세션 제거로 Arc 가 끊겨 자연 종료).
        session.kill(Duration::from_secs(5));

        // 7. 세션 추적 해제(S9 — 좀비 watcher 엔트리 방지). reaper 는 tracker 를 모른다.
        self.tracker.unwatch(agent_id);

        Ok(())
    }

    // ── 조회/종료 ─────────────────────────────────────────────────────────────

    pub fn list_agents(&self) -> Vec<AgentInfo> {
        let sessions: Vec<Arc<AgentSession>> = {
            let guard = self.sessions.read().expect("sessions poisoned");
            guard.values().cloned().collect()
        };
        sessions.iter().map(|s| self.agent_info(s)).collect()
    }

    pub fn get_snapshot(&self, agent_id: AgentId) -> Result<Vec<OutputChunk>, PtyError> {
        let session = self.get_session(agent_id)?;
        Ok(session.snapshot())
    }

    /// list_agents 전체 순회·AgentInfo 조립(profiles lock)을 피해 epoch 만 보는 경량 형제.
    pub fn agent_epoch(&self, agent_id: AgentId) -> Option<u32> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .map(|s| s.epoch)
    }

    pub fn shutdown_all(&self) {
        // ★ADR-0019★: shutting_down 을 각 kill **전에** set 한다. 이게 kill 보다 늦으면 그 틈에
        //   종료된 세션의 finish hook 이 shutting_down=false 를 snapshot 해 KeepDisableAutoRestore 를
        //   맞고(auto_restore=false → 부팅 복원 대상에서 탈락) 마는 race 가 생긴다. set 이 먼저면
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

    /// §10 규칙1 — read lock 을 즉시 해제한 뒤 Arc clone 을 반환한다(호출부는 락 미보유 상태로 이어간다).
    fn get_session(&self, agent_id: AgentId) -> Result<Arc<AgentSession>, PtyError> {
        self.sessions
            .read()
            .expect("sessions poisoned")
            .get(&agent_id)
            .cloned()
            .ok_or(PtyError::NotFound(agent_id))
    }

    /// activate_profile 이 resume 성공 후 산 세션의 info 를 얻는 데 쓴다 — resume_no_fallback 은
    /// 세션을 맵에 등록만 하고 info 를 돌려주지 않으므로(RestoreOutcome 반환) id 로 재조회한다.
    fn agent_info_by_id(&self, id: AgentId) -> Result<AgentInfo, PtyError> {
        let session = self.get_session(id)?;
        Ok(self.agent_info(&session))
    }

    /// 봉투 sender 등 AgentInfo 전체가 필요 없는 호출부가 **agent_info 와 byte-identical** 한 이름을
    /// 얻게 하는 단일 출처다 — session.cwd 기반 resolve 를 여기 한 곳에 모아 로직 복제를 막는다.
    pub fn canonical_name(&self, id: AgentId) -> Option<String> {
        let session = self.get_session(id).ok()?;
        Some(self.resolve_canonical_name(&session))
    }

    /// agent_info·canonical_name 공유 코어 — 이름 파생을 한 곳으로 모아 reaper/ingress/cli 와
    /// 어긋나지 않게 한다.
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
        let cwd = session.cwd.to_string_lossy();
        // get()은 profiles lock 을 잡아 clone 후 즉시 해제한다 — 이 함수 자체가 sessions lock 미보유
        //   상태에서만 호출되므로 두 락을 동시에 잡지 않는다(§10 락 순서).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        crate::agent::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
    }

    /// sessions lock 을 보유하지 않은 상태에서만 호출한다.
    fn agent_info(&self, session: &Arc<AgentSession>) -> AgentInfo {
        // get()은 profiles lock 을 잡아 clone 후 즉시 해제한다 — 이 함수는 sessions lock 미보유 상태에서만
        //   불리므로 두 락을 동시에 잡지 않는다(§10 락 순서).
        let display_name = self.profiles.get(session.id).and_then(|p| p.display_name);
        self.agent_info_with(session, display_name.as_deref())
    }

    /// `agent_info` 의 **표시명 주입형** — 여러 세션분을 만들 때 프로필 스냅샷을 이미 손에 쥔 호출자
    /// (`roster`)가 세션마다 프로필을 다시 뜨지 않게 한다. 이름 파생 규칙 자체는 여기 하나뿐이다
    /// (`canonical_name_or_id_fallback` — 규칙을 복제하지 않는다는 `resolve_canonical_name` 의 규율 그대로).
    fn agent_info_with(
        &self,
        session: &Arc<AgentSession>,
        display_name: Option<&str>,
    ) -> AgentInfo {
        let cwd = session.cwd.to_string_lossy().to_string();
        let name =
            crate::agent::name::canonical_name_or_id_fallback(display_name, &cwd, session.id);
        AgentInfo {
            id: session.id,
            name,
            cwd,
            status: session.status(),
            cols: session.cols.load(Ordering::Relaxed),
            rows: session.rows.load(Ordering::Relaxed),
            epoch: session.epoch,
            capabilities: session.capabilities(),
            // 같은 세션 Arc 에서 뽑는다 — 소비자가 나중에 되묻지 않아도 되게(그 필드 doc).
            reads_messages: session.reads_messages(),
        }
    }
}

impl Drop for AgentManager {
    /// ★명시 Stop 이 필요한 이유★: reaper_tx drop 만으로도 channel 이 닫혀 recv 가 Err 로 끝나지만,
    /// 세션들이 보유한 finish hook 클로저가 reaper_tx clone 을 들고 있어 즉시 안 닫힐 수 있다.
    /// 송신 실패(reaper 가 이미 종료)는 무시한다.
    fn drop(&mut self) {
        let _ = self.reaper_tx.send(ReaperCmd::Stop);
        if let Some(handle) = self.reaper_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn probe_spec() -> CommandSpec {
        CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo select-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        }
    }

    // ── ADR-0044 ──
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

    // ── 회귀 ──
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

    // ── write_stdin_observed_if_epoch ──
    //
    // ★왜 실 spawn 없이 세션을 맵에 직접 꽂나★: 검증 대상은 "맵이 가리키는 세션의 epoch 과 요구 epoch 을
    //   비교해 write 를 집행/거부하는가" 뿐이라, 실 자식·PTY·claude 바이너리가 전부 무관하다(ADR-0012 격리).
    //   in-crate 테스트라 private `sessions` 에 직접 접근한다 — `insert_test_session`(feature gate) 불요.

    use crate::agent::types::{
        ControlCaps, InputCaps, InputEvent, ModelCaps, OutputCaps, SessionCaps, TransportCaps,
    };
    use crate::persistence::{FilePresetStore, FileProfileStore};

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

    /// 같은 id 재삽입 = 재시작(incarnation 교체) 모사.
    fn put_session(manager: &AgentManager, id: AgentId, epoch: u32) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let written = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
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
            true,
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

    // ── subscribe_from: Err 은 on_ready 앞이다(데몬 거절 통보 계약의 전제) ──────────────
    //
    // ★무엇을 지키나★: 데몬은 이 `Err` 를 `AgentEvent::SubscribeFailed` 로 바꿔 보내면서 "이 구독엔 Ack 도
    //   ReplayComplete 도 뒤따르지 않는다"를 광고하고, 클라이언트는 그 광고에 기대어 single-flight 슬롯을
    //   즉시 푼다. `on_ready` 뒤에서 실패하는 갈래가 생기면 거절과 Ack 가 같은 구독에 대해 함께 나가고,
    //   이미 푼 슬롯 위로 늦은 Ack/Complete 가 도착해 replay 가 돌지 않은 세대에 성공 마커가 붙는다.
    //   그 회귀는 런타임에 무신호라(출력이 조용히 죽는다) 이 단언이 유일한 감지기다.
    #[test]
    fn subscribe_from_err_never_invokes_on_ready() {
        struct NoopSink;
        impl OutputSink for NoopSink {
            fn send(
                &self,
                _frame: crate::agent::types::OutputFrame<'_>,
            ) -> Result<(), crate::agent::types::SinkError> {
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                SinkId::nil()
            }
        }

        let manager = bare_manager();
        let missing = AgentId::new_v4(); // 맵에 없는 id — get_session 이 실패한다.
        let mut ready_calls = 0usize;
        let res = manager.subscribe_from(missing, Arc::new(NoopSink), None, false, |_| {
            ready_calls += 1;
        });
        assert!(res.is_err(), "없는 에이전트 구독은 Err");
        assert_eq!(
            ready_calls, 0,
            "Err 경로에서 on_ready(=SubscribeAck) 발행 0"
        );
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
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let old_written = put_session(&manager, id, 0);
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

    /// 프로필 없음 = ad-hoc 산 에이전트. `put_session` 은 cwd 가 `"."` 로 고정이라 이름 축 단언을
    /// 못 해서 따로 둔다.
    fn put_live_session_at(manager: &AgentManager, id: AgentId, cwd: &str) {
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
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
            true,
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

    fn create(manager: &AgentManager, cwd: &str, display_name: Option<&str>) -> AgentProfile {
        manager
            .create_agent(agent_profile(cwd, display_name))
            .expect("이 픽스처는 접미사 공간을 소진시키지 않는다")
    }

    /// ★산 항목의 이름과 계층은 **같은 프로필 스냅샷**에서 나온다★.
    ///
    /// 예전엔 이름이 `list_agents()` 안의 세션별 프로필 조회에서, 계층은 그 뒤의 목록 조회에서 왔다 — 그
    /// 사이에 개명 + 계층 이동이 커밋되면 한 행에 **한 번도 존재한 적 없는 조합**(옛 이름 + 새 부모)이 실린다.
    /// ★이 테스트가 증명하는 것과 못 하는 것★: 두 사실이 프로필 등록부에서 함께 읽힌다는 것은 여기서 본다.
    /// 두 읽기 사이의 창이 **없다**는 것은 구조로 보장되며(조회 자체가 하나뿐이다) 결정적 재현 테스트는
    /// 만들 수 없다 — 재현하려면 이제 존재하지 않는 두 읽기 사이에 seam 을 넣어야 한다.
    #[test]
    fn a_live_roster_row_reads_its_name_and_hierarchy_from_one_profile_snapshot() {
        let manager = bare_manager();
        let lead = create(&manager, "C:/lead", Some("lead"));
        let helper = create(&manager, "C:/helper", Some("helper"));
        put_live_session_at(&manager, helper.id, "C:/live/helper");

        assert!(manager.reparent_agent(helper.id, Some(lead.id)), "전제");
        assert!(
            renamed_ok(manager.rename_agent(helper.id, Some("helper-renamed".into()))),
            "전제"
        );

        let entry = manager
            .roster()
            .into_iter()
            .find(|e| e.id == helper.id)
            .expect("명부에 있어야");
        assert_eq!(entry.canonical_name, "helper-renamed", "새 이름");
        assert_eq!(
            entry.parent,
            Some(lead.id),
            "새 부모 — 옛 이름과 짝지어지지 않는다"
        );
        let live = entry.live.as_ref().expect("살아 있어야");
        assert_eq!(
            live.name, entry.canonical_name,
            "한 파생에서 나온 값이라 항목 안에서 갈릴 수 없다"
        );
        assert_eq!(
            entry.cwd, "C:/live/helper",
            "산 항목의 cwd 는 세션 cwd(ADR-0101)"
        );
    }

    // ── 폭주 백스톱(명부 총량 상한) ──────────────────────────────────────────────────

    /// 상한 테스트 전용 매니저 — 프로필 저장이 **메모리**다. `bare_manager` 의 파일 저장은 등록마다 명부
    /// 전체를 디스크에 쓰므로(락 보유 중 save) 512건 채우기가 O(n²) 디스크 I/O 가 된다. 여기서 보는 것은
    /// 개수 판정이지 영속이 아니다.
    fn capacity_manager() -> AgentManager {
        #[derive(Default)]
        struct MemStore(Mutex<Vec<AgentProfile>>);
        impl crate::agent::profile::ProfileStore for MemStore {
            fn save(&self, profiles: &[AgentProfile]) {
                *self.0.lock().expect("mem store poisoned") = profiles.to_vec();
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.lock().expect("mem store poisoned").clone()
            }
        }
        let tag = uuid::Uuid::new_v4();
        let profiles = Arc::new(crate::agent::profile::ProfileRegistry::new(Arc::new(
            MemStore::default(),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-cap-preset-{tag}")),
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

    /// 상한 직전까지 채운다. 유일성 배정을 거치지 않는 seam 을 쓰는 이유는 속도뿐이다(정상 경로로 채우면
    /// 등록마다 명부를 훑어 O(n²)이 된다).
    fn fill_roster_to(manager: &AgentManager, count: usize) {
        for i in 0..count {
            let mut p = agent_profile(&format!("C:/filler/{i}"), Some(&format!("filler-{i}")));
            p.id = AgentId::new_v4();
            manager.profiles.upsert(p);
        }
        assert_eq!(manager.roster().len(), count, "채움 전제");
    }

    /// ★상한은 입구가 아니라 **등록 커밋 자리**에 있다★ — 그래서 어느 입구로 들어와도 같은 답이다.
    ///   입구에 두었을 때 실제로 새던 두 경로(데스크톱 CreateProfile · ad-hoc spawn 등록)를 여기서 함께 본다.
    #[test]
    fn both_registration_paths_refuse_a_new_agent_at_the_ceiling() {
        let manager = capacity_manager();
        fill_roster_to(&manager, MAX_ROSTER_SIZE - 1);

        // 마지막 한 자리는 통과한다 — 경계가 "근처" 가 아니라 정확히 상한에서 닫힌다.
        let last = manager
            .create_agent(agent_profile("C:/last", Some("last-one")))
            .expect("상한 미만은 통과");
        assert_eq!(manager.roster().len(), MAX_ROSTER_SIZE);

        let err = manager
            .create_agent(agent_profile("C:/over", Some("one-too-many")))
            .expect_err("상한 초과 등록은 거부");
        assert!(
            matches!(
                err,
                PtyError::RosterFull {
                    current: MAX_ROSTER_SIZE,
                    limit: MAX_ROSTER_SIZE
                }
            ),
            "이름 공간 소진과 **구분되는** 전용 신호여야(호출자가 할 일이 다르다): {err}"
        );

        // ad-hoc spawn 의 신규 등록 경로도 같은 답 — 이쪽이 뚫려 있으면 "총량" 이 거짓이 된다.
        let err = manager
            .register_for_spawn(&agent_profile("C:/adhoc", Some("adhoc")))
            .expect_err("ad-hoc 신규 등록도 거부");
        assert!(matches!(err, PtyError::RosterFull { .. }), "{err}");
        assert_eq!(
            manager.roster().len(),
            MAX_ROSTER_SIZE,
            "거부된 등록은 명부를 늘리지 않는다"
        );

        // ★기존 에이전트는 인질이 아니다★: 같은 id 재등록(복원·재spawn)과 개명은 상한에서도 계속 된다.
        manager
            .register_for_spawn(&last)
            .expect("기존 id 재등록은 상한과 무관");
        assert!(
            renamed_ok(manager.rename_agent(last.id, Some("still-renameable".into()))),
            "상한이 명부를 얼려 버리면 복구 자체가 불가능해진다"
        );
    }

    /// ★검사와 커밋이 같은 임계구역★ — 동시 등록이 각자 상한 미만을 관측하고 다 함께 커밋하는 창이 없다.
    #[test]
    fn concurrent_registrations_cannot_all_slip_through_the_last_slot() {
        let manager = Arc::new(capacity_manager());
        fill_roster_to(&manager, MAX_ROSTER_SIZE - 1);

        // 남은 자리는 하나인데 여덟이 동시에 등록을 시도한다.
        let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|s| {
            for i in 0..8 {
                let manager = Arc::clone(&manager);
                let winners = Arc::clone(&winners);
                s.spawn(move || {
                    if manager
                        .create_agent(agent_profile(&format!("C:/racer/{i}"), Some("racer")))
                        .is_ok()
                    {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "빈 자리가 하나면 정확히 하나만 통과해야"
        );
        assert_eq!(manager.roster().len(), MAX_ROSTER_SIZE);
    }

    fn renamed_ok(o: RenameOutcome) -> bool {
        matches!(o, RenameOutcome::Renamed(_) | RenameOutcome::Unchanged(_))
    }

    fn name_of_in_roster(manager: &AgentManager, id: AgentId) -> String {
        manager
            .roster()
            .into_iter()
            .find(|e| e.id == id)
            .expect("명부에 있어야")
            .canonical_name
    }

    fn name_of(manager: &AgentManager, id: AgentId) -> String {
        manager
            .agent_snapshot(id)
            .expect("명부에 있어야")
            .canonical_name_when_live()
    }

    #[test]
    fn roster_reports_live_and_dormant_agents_in_one_query() {
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
        let manager = bare_manager();
        let live_id = AgentId::new_v4();
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
        assert!(renamed_ok(manager.rename_agent(bob.id, Some("bob".into()))));
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "자기 이름 재확정에 접미사가 붙으면 개명할 때마다 번호가 늘어난다"
        );
    }

    #[test]
    fn repeating_a_rename_request_does_not_burn_a_new_number() {
        // ★상류 가드 부재★: 프론트의 "값 안 바뀜" 가드는 현재 이름이 `bob(1)` 이라 재요청에 걸리지
        //   않고, LLM `RenameProfile` 엔 가드가 없다.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let alice = create(&manager, "C:/y", Some("alice"));

        assert!(renamed_ok(
            manager.rename_agent(alice.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, alice.id), "bob(1)");
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
        assert!(
            !manager
                .roster()
                .iter()
                .any(|e| e.canonical_name == "bob(2)"),
            "재요청이 새 번호를 만들면 안 된다: {:?}",
            manager.roster()
        );

        let filler = create(&manager, "C:/f", Some("bob"));
        assert_eq!(filler.canonical_name_when_live(), "bob(2)");
        let carol = create(&manager, "C:/c", Some("carol"));
        assert!(renamed_ok(
            manager.rename_agent(carol.id, Some("bob".into()))
        ));
        assert_eq!(name_of(&manager, carol.id), "bob(3)");
        manager.delete_agent(filler.id);
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
        let manager = bare_manager();
        let a = create(&manager, "C:/shared", None);
        assert_eq!(
            a.canonical_name_when_live(),
            "shared",
            "override 없으면 cwd basename 파생(이 테스트의 전제)"
        );
        let b = create(&manager, "C:/shared", Some("bee"));
        assert_eq!(b.canonical_name_when_live(), "bee");

        assert!(renamed_ok(manager.rename_agent(b.id, None)));
        assert_eq!(name_of(&manager, b.id), "shared(1)");
        assert_eq!(
            manager.agent_snapshot(b.id).unwrap().display_name,
            Some("shared(1)".to_string()),
            "충돌하는 해제는 override 를 없애지 않는다(없애면 동명이 된다)"
        );
        assert!(renamed_ok(manager.rename_agent(b.id, None)));
        assert_eq!(name_of(&manager, b.id), "shared(1)", "해제 재요청도 멱등");
        assert_eq!(name_of(&manager, a.id), "shared");
    }

    #[test]
    fn a_literal_zero_suffix_does_not_occupy_the_unsuffixed_name() {
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob(0)"));

        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "리터럴 bob(0) 은 접미사 없는 bob 을 점유하지 않는다"
        );
        let next = create(&manager, "C:/z", Some("bob"));
        assert_eq!(next.canonical_name_when_live(), "bob(1)");
    }

    #[test]
    fn the_requested_name_is_the_base_verbatim() {
        let manager = bare_manager();
        let one = create(&manager, "C:/x", Some("bob(1)"));
        assert_eq!(
            one.canonical_name_when_live(),
            "bob(1)",
            "비어 있으면 요청한 이름 그대로(계열로 재해석 금지)"
        );
        let plain = create(&manager, "C:/y", Some("bob"));
        assert_eq!(
            plain.canonical_name_when_live(),
            "bob",
            "bob(1) 이 있다고 bob 을 못 쓰게 되면 삭제로 이름을 회수하는 경로가 막힌다"
        );
        let nested = create(&manager, "C:/z", Some("bob(1)"));
        assert_eq!(nested.canonical_name_when_live(), "bob(1)(1)");
        let bob2 = create(&manager, "C:/w", Some("bob"));
        assert_eq!(
            bob2.canonical_name_when_live(),
            "bob(2)",
            "bob 계열 최대는 bob(1) 의 1 뿐이다(bob(1)(1) 은 계열 아님)"
        );
        let nested2 = create(&manager, "C:/v", Some("bob(1)"));
        assert_eq!(nested2.canonical_name_when_live(), "bob(1)(2)");
    }

    #[test]
    fn a_saturated_family_falls_back_to_the_lowest_free_number() {
        // ★포화가 이론이 아니다★: `이름(4294967295)` 은 UI 개명 한 번으로 만들 수 있는 평범한 상태다.
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        create(&manager, "C:/y", Some("bob(4294967295)"));

        let first = create(&manager, "C:/z", Some("bob"));
        assert_eq!(
            first.canonical_name_when_live(),
            "bob(1)",
            "포화여도 빈 번호를 준다(거부하면 계열 전체가 영구 봉쇄된다)"
        );
        let second = create(&manager, "C:/w", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(2)");

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

        assert_eq!(pick_suffix(&set(&[])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, 2])), Some(3));
        assert_eq!(
            pick_suffix(&set(&[2])),
            Some(3),
            "구멍(1)이 있어도 포화가 아니면 내려가지 않는다"
        );
        assert_eq!(pick_suffix(&set(&[u32::MAX])), Some(1));
        assert_eq!(pick_suffix(&set(&[1, u32::MAX])), Some(2));
        assert_eq!(pick_suffix(&set(&[1, 2, 3, u32::MAX])), Some(4));
        assert_eq!(
            pick_suffix(&set(&[2, u32::MAX])),
            Some(1),
            "포화 갈래는 가장 낮은 구멍을 쓴다"
        );
        // ★`None`(계열 전체 점유)은 봉인하지 않는다★ — 1..=u32::MAX 를 다 채운 입력을 만들 수 없다.
    }
    #[test]
    fn epoch_replacement_never_renames_an_existing_agent() {
        let manager = bare_manager();
        let bob = create(&manager, "C:/x", Some("bob"));

        let newcomer = agent_profile("C:/x", Some("bob"));
        manager
            .register_for_spawn(&newcomer)
            .expect("신규 등록 성공");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "명부에 없던 id = 신규 등록 → 접미사"
        );

        manager.register_for_spawn(&bob).expect("재등록 성공");
        assert_eq!(
            name_of(&manager, bob.id),
            "bob",
            "재시작이 이름을 바꾸면 안 된다"
        );
        manager.register_for_spawn(&newcomer).expect("재등록 성공");
        assert_eq!(
            name_of(&manager, newcomer.id),
            "bob(1)",
            "재등록은 접미사를 누적하지 않는다"
        );

        let third = create(&manager, "C:/x", Some("bob"));
        assert_eq!(third.canonical_name_when_live(), "bob(2)");

        // ★★stale 스냅샷 재등록 — 실전 회귀 시나리오★★: 산 세션 도중 트리에서 개명 → 그 뒤 재시작이
        //   **개명 전 스냅샷**을 들고 재등록하는데, 그 옛 이름을 그사이 **다른 에이전트가 차지**했다.
        //   이때 신규-등록 검사가 돌면 옛 이름이 충돌로 판정돼 재시작이 에이전트를 엉뚱한 이름으로 개명한다.
        let agent = create(&manager, "C:/stale", Some("was-here"));
        let stale_snapshot = agent.clone();
        assert!(renamed_ok(
            manager.rename_agent(agent.id, Some("renamed".into()))
        ));
        assert_eq!(name_of(&manager, agent.id), "renamed");
        let squatter = create(&manager, "C:/squat", Some("was-here"));
        assert_eq!(squatter.canonical_name_when_live(), "was-here");
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
        let manager = bare_manager();
        let first = create(&manager, "C:/x", Some("bob"));
        let second = create(&manager, "C:/y", Some("bob"));
        assert_eq!(second.canonical_name_when_live(), "bob(1)");

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
        let manager = bare_manager();
        create(&manager, "C:/x", Some("bob"));
        let x = create(&manager, "C:/somewhere/zeta", None);
        assert_eq!(x.canonical_name_when_live(), "zeta", "프로필 파생 축");
        put_live_session_at(&manager, x.id, "C:/live/bob(1)");
        assert_eq!(name_of_in_roster(&manager, x.id), "bob(1)", "명부 축");

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
        //   명부엔 쓸모없는 문자열이 남는다.
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
        let manager = bare_manager();
        let a = create(&manager, "C:/x", Some("  bob smith  "));
        assert_eq!(a.display_name, Some("bob smith".to_string()));
        assert_eq!(name_of(&manager, a.id), "bob smith");
    }

    #[test]
    fn renaming_to_a_padded_form_of_the_current_name_burns_no_number() {
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
        let next = create(&manager, "C:/b", Some("bob"));
        assert_eq!(
            next.canonical_name_when_live(),
            "bob(1)",
            "패딩 개명이 bob 을 비웠으면 여기서 동명 두 건이 앉는다"
        );
    }

    #[test]
    fn a_padded_request_for_a_taken_name_gets_the_suffixed_form() {
        let manager = bare_manager();
        create(&manager, "C:/h", Some("bob"));

        let other = create(&manager, "C:/o", Some("  bob  "));
        assert_eq!(
            other.display_name,
            Some("bob(1)".to_string()),
            "패딩 요청도 접미사 계열로 들어간다"
        );
        assert_eq!(name_of(&manager, other.id), "bob(1)");

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

    /// ★왜 훅 지점이 `capabilities()` 인가★: `roster()` → `list_agents()` → `agent_info()` 가 세션마다
    ///   그걸 부른다. 즉 `rename_agent` 의 **관측 도중** 임의 코드를 끼울 수 있는 유일한 주입점이라,
    ///   "커밋 직전에 프로필이 사라지는" 창을 스레드·타이밍 없이 결정적으로 재현할 수 있다.
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

    fn put_live_session_with(
        manager: &AgentManager,
        id: AgentId,
        cwd: &str,
        transport: Box<dyn AgentTransport>,
    ) {
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
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
            true,
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
        let manager = bare_manager();
        let victim = create(&manager, "C:/victim", Some("before"));
        let vid = victim.id;

        let profiles = Arc::clone(&manager.profiles);
        put_live_session_with(
            &manager,
            vid,
            "C:/live/victim",
            Box::new(HookedTransport {
                hook: Mutex::new(Some(Box::new(move || profiles.remove(vid)))),
            }),
        );

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
        let manager = bare_manager();
        let y = create(&manager, "C:/other", Some("q"));
        // X: override "x", 프로필 축 basename "p", 산 축 basename "q"(= Y 와 충돌하는 쪽).
        let x = create(&manager, "C:/prof/p", Some("x"));
        put_live_session_at(&manager, x.id, "C:/live/q");
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "전제 — override 가 이름"
        );

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
        // 이 픽스처는 두 축의 갈림을 `<temp>/.` 로 직접 만든다 — raw basename 은 `"."`, canonicalize 하면
        //   `<temp>` 의 마지막 세그먼트다.
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
        let y = create(&manager, "C:/other", Some(&raw_base));
        let x = create(&manager, "C:/prof/whatever", Some("x"));
        put_live_session_at(&manager, x.id, &dotted);
        assert_eq!(
            name_of_in_roster(&manager, x.id),
            "x",
            "전제 — override 가 이름"
        );

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

    /// ★실 프로세스로 보는 이유★: 표식 발급은 `spawn_agent` 안에 있고, 그 자리를 타는지는 실 spawn 만이
    /// 말한다.
    #[cfg(windows)]
    #[test]
    fn a_fresh_respawn_of_a_reaped_agent_never_reuses_the_prior_epoch() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("epoch-reuse"),
        );
        let first = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("첫 spawn");
        manager.kill_agent(first.id).ok();
        // reaper 가 맵에서 수거할 때까지 — 그 뒤라야 두 번째 spawn 이 이중 spawn 가드를 통과한다.
        let reaped = (0..200).any(|_| {
            if manager.list_agents().is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
            false
        });
        assert!(reaped, "kill 후 세션이 수거돼야(전제)");

        let second = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("Fresh 재spawn");
        assert_ne!(
            second.epoch, first.epoch,
            "Fresh 재spawn 이 죽은 화신의 표식을 재사용하면 안 된다"
        );
        manager.kill_agent(second.id).ok();
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

        let adhoc = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("bob"));
        let second = manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect("ad-hoc spawn");
        assert_eq!(second.name, "bob(1)", "신규 등록 spawn 은 접미사를 받는다");

        manager.kill_agent(first.id).ok();
        manager.kill_agent(second.id).ok();
    }
}
