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
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend;
use crate::failure::AgentFailureKind;
use crate::output_core::{OutputCore, TurnWiring};
use crate::preset::PresetRegistry;
use crate::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, RestoreOutcome, RestoreReport, SpawnMode,
};
use crate::reaper::{self, ReaperCmd, ReaperDeps};
use crate::session::AgentSession;
use crate::session_tracker::SessionTracker;
use crate::transport::{LinkResolution, LinkSink};
use crate::turn::TurnObservations;
use crate::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, ControlChannel, NoopControlChannel, OutputChunk,
    OutputEvent, OutputSink, PtyError, ReapMsg, SinkId, StatusSink, SubscribeOutcome,
    TerminalReason, TerminationIntent,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// resume spawn 후 이 시간 안에 비정상 종료(code≠0/Failed/Killed)하거나 **진단 스트림이 실패를 말하면**
/// resume 실패로 판정한다(H-1.7 "조기 종료 윈도"). 성공한 resume은 TUI라 계속 떠 있다.
///
/// ★늘려서 문제를 풀지 마라(실측 2026-08-23)★: 실 claude 는 진단을 +2.2s 에 내고 종료는 +6.4s 에야
///   한다 — 죽음만 기다리는 판정은 이 창을 6s 넘게 늘려야 맞는데, 활성화는 이 창만큼 **블록**하므로
///   그러면 멀쩡한 이어받기가 전부 그만큼 느려진다. 그래서 판정 근거를 「죽음」에서 「죽음 또는 증거」로
///   바꿨다(`EarlyVerdict`) — 오히려 실패를 더 **빨리** 확정한다.
// ADR-0172
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(3);

/// 통로가 **연결의 결말을 아예 안 내는** 경우에만 쓰이는 liveness 백스톱.
///
/// ★★이것은 판정 창이 아니다 — 판정은 연결 그 자체다★★(사용자 결정): 이어받기의 성공은 「N 초 안에 안
///   죽었다」가 아니라 **「에이전트가 입력을 받을 준비가 됐다고 신호했다」**이고, codex 에서 그 신호는
///   핸드셰이크 완료(`Link::Up`)다. 거절도 상대가 **즉시** 답한다(실측). 그래서 양방향 모두 기다릴 것이
///   없고, [`AgentManager::early_activation_verdict`] 는 연결이 결말을 내는 **그 순간** 돌아온다.
///   ★한때 여기 있던 「연결 판정 창」(15 초를 끝까지 기다리는 정책)은 그 결정이 걷어냈다 — 되살리지 말 것★.
///
/// ★그럼에도 이 값이 남아 있는 이유는 **단 하나**다★: 통로가 결말을 **못 내는** 경로가 실재한다.
///   [`crate::backend::codex`] 의 핸드셰이크 상한은 **응답 대기**에만 걸리고 **쓰기**에는 안 걸린다 —
///   상대가 우리 stdin 을 읽지 않으면 `write_all` 이 파이프 backpressure 로 무한히 매달리고, 그러면
///   라이터가 실패 갈래에 도달하지 못해 링크가 `Connecting` 에 영영 멈춘다. 그 상태에서 이 백스톱이
///   없으면 `activate_profile` 이 **영원히** 블록한다(데몬의 async dispatch 워커 하나가 통째로 묶인다).
/// ★그래서 이 값이 하는 일은 「판정」이 아니라 「포기」다★ — 여기 닿았다는 것은 상대가 답도 안 하고
///   우리 쓰기도 안 받는다는 뜻이고, 그 결말은 실패이자 **아래 teardown 의 트리거**다(그 kill 이 매달린
///   쓰기까지 함께 푼다).
/// ★값은 통로의 핸드셰이크 상한보다 커야 한다★ — 작으면 통로가 스스로 실패를 확정하기 **전에** 이쪽이
///   가로채, 사유 없는 포기가 사유 있는 거절을 덮는다. 그 대소는 시험대가 직접 잰다.
pub(crate) const LINK_RESOLUTION_BACKSTOP: Duration = Duration::from_secs(15);
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

/// spawn 요청 하나의 결말. ★"띄웠다" 와 "할 일이 없었다" 를 **호출자가 구분할 수 있어야 한다**★ —
/// 그래야 등록·epoch·기록 같은 뒷정리를 자기가 만들지 않은 세션에 하지 않는다.
///
/// ★중복 요청은 오류가 아니다(설계 결정 — 되돌리지 마라)★: 그 항목이 이미 떠 있거나 이미 뜨는 중이면
///   이 요청은 **무의미(moot)** 하다. 예전엔 그것을 `Err` 로 답했고, 그래서 소비자마다 "이 오류는 진짜
///   실패가 아니다" 를 알아야 했다 — 그 목록을 세 번 손봤고 세 번 다 하나씩 빠졌다. 오류가 아니라 결말로
///   두면 빠뜨릴 목록 자체가 없어진다. 프론트는 이미 같은 모양이다(`AgentList` 의 in-flight 가드가 두 번째
///   활성화를 조용히 흘린다).
/// ★moot 은 아무것도 바꾸지 않는다★: 한 것이 없으므로 기록할 실패도, 지울 근거도 없다
///   (`AgentManager::note_spawn_result`).
// ADR-0172
#[derive(Debug, Clone)]
pub enum SpawnOutcome {
    /// 이 호출이 화신을 띄웠다.
    Started(AgentInfo),
    /// **이 요청은 할 일이 없었다** — 그 항목이 이미 떠 있거나 이미 뜨는 중이라 아무것도 하지 않았다.
    ///
    /// `Some` = 그 순간 산 세션을 볼 수 있었다(그 정보를 그대로 돌려준다) · `None` = 승자가 아직 명부에
    /// 올리기 전이라 볼 것이 없었다. 어느 쪽이든 **이 호출은 아무것도 만들지 않았다**.
    Moot(Option<AgentInfo>),
}

impl SpawnOutcome {
    /// 이 호출이 실제로 띄웠나 — 뒷정리를 할 **자격**의 판정.
    pub fn started(&self) -> bool {
        matches!(self, SpawnOutcome::Started(_))
    }

    /// 결말과 무관하게 "지금 그 에이전트" 정보. moot 은 남이 띄운 세션이라 `None` 일 수 있다.
    pub fn into_info(self) -> Option<AgentInfo> {
        match self {
            SpawnOutcome::Started(info) => Some(info),
            SpawnOutcome::Moot(info) => info,
        }
    }

    /// **이 호출이 띄운** 화신만. moot 이면 `None` — "내가 만든 것" 에만 뒷정리를 하려는 호출자용이다.
    pub fn into_started(self) -> Option<AgentInfo> {
        match self {
            SpawnOutcome::Started(info) => Some(info),
            SpawnOutcome::Moot(_) => None,
        }
    }
}

/// 실패 분류가 들여다보는 출력 꼬리의 하드 상한(`OutputCore::terminal_tail` 이 정확히 지킨다). 우리가
/// 찾는 종료 문구는 마지막 몇 줄에 있다. 이 값은 화면·로그로 나가지 않는다(분류 입력 전용).
// ADR-0172
const FAILURE_TAIL_BYTES: usize = 4096;

/// 이어받기 시도의 조기 판정 결말 — 「죽었나」가 아니라 「무슨 증거가 섰나」로 갈린다.
///
/// ★`Diagnosed` 가 있는 이유(실측 2026-08-23 — 되돌리지 마라)★: 실 claude 는 `--resume <빈 sid>` 에
///   대해 **+2.2s 에 진단을 내고 +6.4s 에야 죽는다**(그 사이 SessionEnd 훅이 돈다). 죽음만 기다리면
///   조기종료 창(3s)을 넘겨 **실패가 성공으로 판정되고 앞선 기록까지 지워진다** — 이 기능의 표제
///   사례가 통째로 죽는 자리였다. 증거가 이미 섰으면 시체를 기다리지 않는다.
/// ★창을 늘리는 것은 이 문제의 답이 아니다★: 활성화는 이 창만큼 **블록**하므로, 늘리면 멀쩡한
///   이어받기가 전부 그만큼 느려진다. 답은 기다림이 아니라 판단 근거를 바꾸는 것이다.
// ADR-0172
#[derive(Debug)]
enum EarlyVerdict {
    /// 창 안에 종점 상태로 들었다. `evidence` = 그 순간 붙잡은 콘솔 꼬리 + 진단 텍스트(best-effort,
    /// 빈 값이 정상).
    Terminal {
        status: AgentStatus,
        evidence: String,
    },
    /// **아직 살아 있지만** 진단 스트림이 이미 알려진 실패를 말했다 — 죽음을 기다리지 않고 확정한다.
    Diagnosed(AgentFailureKind),
    /// 창을 넘겼고 아무 증거도 서지 않았다 = 활성화 성립.
    ///
    /// ★이것은 **약한** 성공 증거다★ — 「N 초 안에 안 죽었다」일 뿐 그 에이전트가 입력을 받을 수 있는지는
    ///   모른다. 연결 축이 없는 통로(claude·shell·stdio)에는 이것뿐이라 그대로 쓴다.
    Alive,
    /// 에이전트가 **입력을 받을 준비가 됐다고 신호했다** = 활성화 성립(사용자 결정).
    ///
    /// ★`Alive` 와 갈라 두는 이유가 두 가지다★:
    ///   ① **증거의 종류가 다르다** — 이쪽은 상대가 낸 긍정 신호(codex 핸드셰이크 완료)이고, 저쪽은
    ///      아무 일도 안 일어났다는 관측이다. 그래서 이쪽은 기다릴 것이 없다(신호가 오면 그 즉시 끝).
    ///   ② **「마지막 실패」 처분이 다르다** — 이 낙관적 성공은 기록을 **지우지 않는다**(사용자 조건).
    ///      지움 규칙의 정본은 `AgentManager::resume_no_fallback` 의 그 자리다.
    Ready,
    /// 통로가 **연결을 못 세웠다** — 프로세스는 살아 있을 수 있다.
    ///
    /// ★`Terminal` 과 갈라 두는 이유★: 저쪽은 「죽었다」이고 이쪽은 「살아 있는데 못 쓴다」다. 처분은
    ///   같은 실패지만 사유 문장과 분류 입력이 다르고, 무엇보다 **이 갈래에는 시체가 없다** — 콘솔
    ///   꼬리도 진단 꼬리도 비어 있고 증거는 `reason` 하나뿐이다.
    LinkFailed { reason: String },
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

/// 「backend 가 받아 온 세션 id」를 이 프로필의 이 화신에 적는 한 동사를 만든다
/// ([`backend::SessionIdSink`] 의 조립점 쪽 실물).
///
/// ★`String` → `Uuid` 해독이 여기 있는 이유★: 상대가 주는 것은 문자열이고(codex `Thread.id`) 프로필이
///   드는 것은 `Uuid` 다. 그 폭을 좁히는 것은 프로필 스키마 지식이라 `backend/` 에 둘 수 없다(ADR-0004).
/// ★**여기로 들어오는 값이 v4 라고 가정하지 말 것 — 실측은 UUIDv7 이다**★: 실 app-server 가 준 것은
///   `01a0a08f-…`(버전 니블 `7`)였다. 그래서 `backend_session_id` 한 칸에 **백엔드마다 다른 UUID 버전**이
///   앉는다 — claude 는 우리가 v4 를 뽑아 건네고, codex 는 받아 적는다. 이 저장소의 시험대는 전부
///   `Uuid::new_v4()` 로 값을 만들어서 그 차이를 한 번도 겪지 않는다. ★버전 검증을 넣지 말 것★ —
///   `Uuid::parse_str` 은 버전을 안 보고, 그것이 이 경로가 두 버전 다 받는 이유다.
/// ★uuid 로 못 읽히면 로그로 남기고 **버린다**★ — panic 도 `unwrap` 도 아니다. 이 호출은 라이터 스레드
///   위에서 돌고 그 스레드는 핸드셰이크·입력 전송을 함께 지므로, 여기서 죽으면 상대 형식이 한 번 바뀐
///   것만으로 그 세션이 통째로 말을 잃는다. 기록만 못 한 것이 낫다(그러면 이어받기가 Fresh 로 떨어져
///   정직하게 보고된다).
/// ★값 자체를 로그에 싣지 않는다★ — 남기는 것은 해독 실패 사유뿐이다.
/// ★화신 표식을 `Some` 으로 못 박는다(ADR-0007/0163)★ — 이 동사는 **한 spawn** 에 묶여 있고 그 spawn 의
///   표식은 호출 시점에 이미 확정돼 있다. 그래서 **더 새 화신이 이미 선** 뒤에 도착한 기록은 거절된다.
///   ★「늦은 기록을 막는다」로 넓혀 읽지 말 것★ — 세션이 그냥 **끝나기만** 한 경우는 표식이 그대로라
///   이 가드가 안 선다(그 창의 정본 = 통로의 `record_session_id` doc).
/// ★세 결말이 로그로 갈린다(`docs/reference/logging-conventions.md` 「계측 의무」)★ — 기록됨(info) ·
///   버려짐(debug, S14 stale 가드와 같은 자리) · uuid 해독 실패(warn). **받지 못한 결말**은 여기 오지
///   않고 통로의 핸드셰이크 실패 로그가 낸다.
/// ★`true` 를 「영속됐다」로 읽지 말 것★ — 그 값이 뜻하는 것은 메모리 명부가 바뀌었다는 것뿐이고, 디스크
///   쓰기 실패는 저장소가 자기 자리에서 `error!` 로 낸다([`ProfileStore::save`] 는 `()` 를 돌려준다).
// ADR-0007
// ADR-0185
/// 통로가 배달한 연결 결말 한 건 — ★어느 화신의 것인지를 **함께** 나른다★.
///
/// ★표식이 값에 붙어 있는 것이 이 타입의 요점이다★: 결말과 화신이 따로 다니면, 수거된 화신의 결말이
///   막 뜬 후임에게 적용된다(그 정리가 **산 세션을 죽인다**). 소비자는 이 표식을 **일치/불일치로만**
///   본다 — 대소로 「더 새 것」을 유도하지 않는다(ADR-0163/0164).
#[derive(Debug, Clone)]
pub(crate) struct LinkVerdict {
    pub(crate) epoch: u32,
    pub(crate) resolution: LinkResolution,
}

/// 연결의 결말을 기다리는 호출자가 쥐는 한 벌 — **배달함과 spawn 예약이 한 몸**이다.
///
/// ★두 칸을 묶은 것이 요점이다★: 연결을 선언하는 통로에서는 「세션이 명부에 올랐다」와 「그 세션에
///   입력을 보낼 수 있다」 사이가 핸드셰이크 한 왕복만큼 벌어진다. 그 구간에 들어온 두 번째 활성화가
///   명부만 보고 「떠 있다」로 답하면, 첫 요청이 실패로 판정해 그 화신을 거두는 순간 **두 호출자가
///   서로 다른 사실을 들고 갈린다**(뒤엣것은 자기 밑에서 죽을 에이전트를 산 것으로 보고한다).
///   예약을 결말까지 들고 있으면 그 구간의 답이 「이미 뜨는 중」으로 하나가 된다.
/// ★그래서 이 값을 **버리면 예약이 즉시 풀린다**★ — [`AgentManager::spawn_agent`] 가 그렇게 한다.
///   그쪽은 결말을 기다리지 않는 갈래라 예약을 들고 있을 근거도 없다.
/// ★연결 축이 없는 통로에는 이 타입이 아예 오지 않는다★(`Option` 이 `None`) — 그 경로는 예약 수명도
///   판정도 예전 그대로다.
struct LinkWatch {
    rx: Receiver<LinkVerdict>,
    /// 결말이 날 때까지 놓지 않는다 — 읽지 않는 것이 정상이다(수명 하나가 이 칸의 전부다).
    _reservation: SpawnReservation,
}

/// 통로에 건네줄 배달 포트를 만든다 — 화신 표식을 **여기서** 찍어 채널로 넘긴다.
///
/// ★통로는 화신을 모른다★([`session_id_sink`] 와 같은 모양·같은 사유): 표식은 조립점 개념이고, 통로에
///   넣으면 그 지식이 backend 층으로 샌다(ADR-0004).
/// ★보내기 실패를 삼키는 것이 의도다★ — 받는 쪽이 이미 떠났다는 뜻(판정이 끝났거나 Fresh spawn 이라
///   애초에 아무도 안 기다린다)이고, 그때 통로가 할 수 있는 일도 알아야 할 일도 없다.
fn link_verdict_sink(tx: Sender<LinkVerdict>, incarnation: u32) -> LinkSink {
    Arc::new(move |resolution: LinkResolution| {
        let _ = tx.send(LinkVerdict {
            epoch: incarnation,
            resolution,
        });
    })
}

/// spawn 도중 프로필이 사라졌다 — **spawn 을 중단한다**. `at` = 어느 단계에서 알아챘나.
///
/// ★단계 이름을 싣는 이유★: `spawn_agent` 에는 이 판정 자리가 **셋**이다(화신 표식 확정 · 세션 id 발급 ·
///   이어받기 손잡이 읽기). 어느 자리가 걸렸는지가 그 삭제가 언제 끼어들었나를 말해 주는데, 문구가 같으면
///   로그·오류에서 구별되지 않는다.
/// ★셋 다 같은 처분인 것이 규율이다★ — 하나라도 `None` 을 삼키면 삭제된 프로필로 세션이 뜨거나
///   이어받기가 조용히 새 대화가 된다.
fn profile_vanished_mid_spawn(id: AgentId, at: &str) -> PtyError {
    PtyError::SpawnFailed(format!(
        "profile {id} vanished mid-spawn at [{at}] (concurrent delete) — spawn aborted"
    ))
}

fn session_id_sink(
    profiles: Arc<ProfileRegistry>,
    id: AgentId,
    incarnation: u32,
) -> backend::SessionIdSink {
    Arc::new(move |raw: &str| {
        // ★기록 호출을 match 가드에 두지 말 것★ — 부작용이 있는 가드는 앞에 팔 하나만 끼어도 호출 횟수가
        //   조용히 0 이나 2 가 된다. 판정과 분기를 갈라 둔다.
        let sid = match uuid::Uuid::parse_str(raw) {
            Ok(sid) => sid,
            Err(e) => {
                tracing::warn!(
                    agent = %id,
                    epoch = incarnation,
                    "backend 가 준 세션 id 를 uuid 로 읽지 못해 버린다: {e}"
                );
                return;
            }
        };
        if profiles.observe_session_id(id, Some(incarnation), sid) {
            tracing::info!(
                agent = %id,
                epoch = incarnation,
                "backend 가 준 세션 id 를 프로필 명부에 반영했다"
            );
        } else {
            // ★사유를 가르지 못한다 — 돌아오는 것이 `bool` 하나다★: 화신 표식 불일치 · 프로필 부재 ·
            //   이미 같은 값, 셋이 같은 `false` 로 온다. 지어내지 않고 그대로 적는다.
            tracing::debug!(
                agent = %id,
                epoch = incarnation,
                "backend 가 준 세션 id 가 명부에 반영되지 않았다 — 화신 표식 불일치·프로필 부재·같은 값 중 하나"
            );
        }
    })
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

    pub fn agent_backend_session_id(&self, id: AgentId) -> Option<uuid::Uuid> {
        self.profiles.get(id).and_then(|p| p.backend_session_id)
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
            Some(live_cwd) => {
                crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &live_cwd, id)
            }
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
        //
        // ★★「있나」와 「쓴다」를 **한 임계구역**에서 한다 — 두 호출로 되돌리지 말 것★★:
        //   조회와 쓰기가 갈리면 그 사이에 낀 `DeleteProfile` 이 무시되고, **삭제가 성공으로 보고된 뒤
        //   그 프로필이 되살아나며 자식 프로세스까지 따라 뜬다.** 그래서 갱신은
        //   [`ProfileRegistry::update_preserving_hierarchy`] 가 락 안에서 존재까지 함께 판정하고,
        //   여기서는 그 결과로만 갈린다.
        // ★`false` = 그 사이 지워졌다 → **spawn 을 끊는다**★: 아래 신규 등록 갈래로 떨어뜨리면 그게 바로
        //   부활이고, 그냥 진행하면 프로필 없는 세션이 명부에 오른다.
        //   ★뒤에 선 `profile_vanished_mid_spawn` 검사 셋과 **같은 처분, 다른 시점**이다★ — 그쪽은 표식
        //   확정 **뒤**의 삭제를 잡고 이쪽은 그 앞을 잡는다. 어느 하나로 대체되지 않는다.
        // ★**이 줄이 부활 경로를 전부 닫지는 않는다**★ — 등록이 시작되기 **전에** 삭제가 끝났으면 이
        //   조회가 애초에 없어서 아래 신규 등록(ad-hoc spawn) 갈래로 간다. 그 둘을 가르려면 호출자가
        //   「원래 있던 항목이다」를 들고 와야 하는데 그 신호가 `spawn_agent` 시그니처에 없다(미해결).
        if self.profiles.get(profile.id).is_some() {
            return if self.profiles.update_preserving_hierarchy(profile.clone()) {
                Ok(())
            } else {
                Err(profile_vanished_mid_spawn(profile.id, "spawn 등록"))
            };
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

    /// ★두 갈래의 「할 일 없음」은 `Ok(Moot)` 다 — `Err` 로 되돌리지 마라★: 그 항목이 이미 떠 있거나
    /// (아래 이중-spawn 가드) 이미 뜨는 중이면(예약 패배) 이 요청은 무의미하고, 무의미는 실패가 아니다.
    /// 근거와 그 전환이 없앤 것은 `SpawnOutcome` 주석이 갖는다.
    // ADR-0172
    /// ★배달 채널을 버리는 얇은 위임★ — 연결 결말을 기다리는 호출자는 아래
    /// [`AgentManager::spawn_agent_watching_link`] 를 쓴다. 이 갈래에서 채널이 닫히면 통로의 배달은
    /// 조용히 실패하고(그 포트가 그렇게 설계됐다) 아무도 기다리지 않는다.
    /// ★그래서 **이 동사로 띄운 화신의 연결 실패는 주인이 없다**★ — 활성화 입구
    /// ([`AgentManager::activate_profile`]·[`AgentManager::restore_one`])는 Fresh 도 Resume 도
    /// [`LinkWatch`] 를 쥐는 갈래로 들어가므로 그 구멍에 닿지 않는다.
    /// ★남는 소비자는 데몬의 **즉석 by-cwd 생성** 하나다 — ★한때 여기 「WS `Spawn`」으로 적혀 있었는데
    ///   그건 거짓이다★: 그 명령은 `activate_profile` 로 들어간다(그 자리 주석이 「활성화 입구는 하나여야
    ///   한다」로 그렇게 못박았다). 실제로 이 동사를 직접 부르는 운영 자리는 프로필을 등록하지 않고 즉석
    ///   생성하는 그 갈래뿐이고, **오늘 거기 닿는 codex 는 출력 형식이 터미널이라 연결 축이 없다**
    ///   (근거의 정본 = `backend/codex/transport.rs` 의 같은 구멍 주석).
    pub fn spawn_agent(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<SpawnOutcome, PtyError> {
        self.spawn_agent_watching_link(profile, mode)
            .map(|(o, _)| o)
    }

    fn spawn_agent_watching_link(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<(SpawnOutcome, Option<LinkWatch>), PtyError> {
        // ★★예약을 **명부 조회보다 먼저** 잡는다 — 순서를 되돌리지 마라★★: 연결을 선언하는 통로에서는
        //   예약이 spawn 이 끝날 때까지가 아니라 **연결의 결말이 날 때까지** 살아 있다([`LinkWatch`]).
        //   그 구간의 세션은 명부에 올라 있지만 아직 **입력을 받을 수 있는지 모르는** 화신이다. 조회를
        //   먼저 두면 그 화신을 본 두 번째 요청이 `Moot(Some(..))` — 즉 「떠 있다」 — 로 답하는데, 그
        //   답이 곧 결함이다: 첫 요청이 곧 실패로 판정해 그것을 거두면, 두 번째 요청의 호출자는 **자기
        //   밑에서 죽을 에이전트를 산 것으로 들고 있게 된다.**
        //   예약을 먼저 보면 그 구간의 답이 `Moot(None)`(「이미 뜨는 중」)이 되어 두 호출자가 갈리지 않는다.
        //   ★연결 축이 없는 통로에서는 이 순서가 아무것도 바꾸지 않는다★ — 거기서는 예약이 이 함수와
        //   함께 끝나므로, 「떠 있다」와 「뜨는 중」이 겹치는 구간이 애초에 명령 몇 개 폭이다.
        let Some(reservation) = SpawnReservation::reserve(self.spawning.clone(), profile.id) else {
            // 승자가 아직 명부에 올리기 전이거나, 올렸어도 그 연결이 아직 결말을 못 냈다 — 어느 쪽이든
            // 우리가 「떠 있다」고 답할 근거가 없다.
            tracing::info!(
                agent = %profile.id,
                "spawn_agent: 이미 뜨는 중 — 이 요청은 할 일이 없다(moot)"
            );
            return Ok((SpawnOutcome::Moot(None), None));
        };

        // ★잔여 레이스(ADR-0082 미해결·후속)★: 이 이중-spawn 가드는 여기서 read lock 을 잡아 contains_key 를 본 뒤
        //   놓고, 실제 등록(sessions.insert)은 아래에서 별개 write lock 으로 한다 — 그 사이 창이 있다.
        //   같은 id 를 **서로 다른 연결**이 동시에 SpawnProfile 하면 둘 다 이 검사와 activate_profile 의
        //   pre-check 를 통과해 double-spawn 이 날 수 있다(데몬 명령 처리는 연결당 직렬일 뿐 연결 간엔
        //   아니다 — 각 연결이 제 read_task 에서 dispatch 를 await 한다). 이 window 는 ADR-0082 이전부터
        //   있던 **선재(pre-existing) 레이스**이며 이번 변경이 도입하지도 닫지도 않았다(후속 과제로 flag).
        //   ★위 예약이 그 폭을 **줄이지만 닫지는 않는다**★ — 예약 취득과 이 조회 사이에도 같은 창이 있다.
        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "spawn_agent: 이미 실행 중 — 이 요청은 할 일이 없다(moot)"
            );
            return Ok((SpawnOutcome::Moot(Some(self.agent_info(&session))), None));
        }

        self.register_for_spawn(profile)?;

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
        //
        // ★**`register_for_spawn` 바로 뒤여야 한다 — 사이에 한 줄도 끼우지 말 것**★:
        //   [`ProfileRegistry::upsert_preserving_hierarchy`] 는 live 표식을 **보존**하므로(ADR-0084 —
        //   스냅샷이 표식을 author 하면 안 되기 때문), 이 줄이 돌기 전까지 명부에 서 있는 표식은 여전히
        //   **앞 화신의 것**이다. 그 구간에 도착한 앞 화신의 지각 기록
        //   ([`ProfileRegistry::observe_session_id`] 의 `Some(epoch)` 갈래)은 표식이 일치하므로 **거절되지
        //   않고 통과한다** — 그 가드가 막는 것은 「더 새 화신이 이미 섰다」 하나뿐이고, 새 화신은 아직
        //   안 섰다. 그래서 이 줄이 **그 구간을 닫는 동사**다.
        //   ★그 구간이 실제로 무엇을 망가뜨리나(「그냥 낡은 값」이 아니다)★: 아래 sid 발급이 이 줄
        //   **뒤**에 있으므로, Fresh spawn 이 갓 뽑은 uuid 를 앞 화신의 지각 기록이 덮고 그 새 값을
        //   이력으로 밀어낼 수 있다 — 프로세스는 `--session-id <새 값>` 으로 떴는데 프로필은 죽은 화신의
        //   값을 들게 된다. 옛 자리(cwd 정규화와 sid 발급 **뒤**)에서는 그 구간이 canonicalize 한 번 +
        //   `agents.json` 통째 쓰기 한 번만큼 벌어져 있었다.
        //   ★**그 대가로 「프로필이 사라졌다」를 보는 자리가 늘었다 — 아래 둘도 `?` 로 끊는다**★:
        //   이 줄이 맨 앞으로 오면서, 여기서 통과한 뒤 sid 발급·명부 읽기 **사이**에 프로필이 지워지는
        //   창이 생겼다. 그 창을 안 막으면 발급과 읽기가 조용히 `None` 을 돌려주고 spawn 은 계속 가서,
        //   이어받기가 말없이 새 대화가 되고 프로필 없는 세션이 명부에 오른다. 옛 배치에서는 이 줄이
        //   맨 뒤라 그 인터리빙이 여기서 걸렸다 — 지금은 세 자리가 각자 건다.
        // ADR-0007
        let epoch = self
            .profiles
            .epoch_for_spawn(profile.id)
            .ok_or_else(|| profile_vanished_mid_spawn(profile.id, "화신 표식 확정"))?;

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
        //   ★발급 축 단독으로 판정한다(ADR-0185)★: 「우리가 sid 를 뽑아 건네주나」와 「저장된 sid 로
        //   이어받을 수 있나」는 다른 질문이다. 뒤엣것으로 여기를 가르면, 자기 id 를 스스로 발급하는
        //   프로그램에 **그 프로그램이 한 번도 쓰지 않을 uuid** 가 발급돼 프로필에 영속된다.
        // ★`None` 은 「발급 안 함」이 아니라 「프로필이 사라졌다」다 — 삼키지 말고 끊는다★: 이 두 동사는
        //   프로필이 있으면 반드시 값을 돌려준다(`m.get_mut(&id)?` 하나만이 `None` 을 만든다). 위 표식
        //   확정을 통과한 뒤 지워진 경우가 여기로 오는데, 그대로 `None` 으로 흘리면 발급 축이 켜진
        //   backend 가 **sid 없이** 떠서 Resume 이 말없이 새 대화가 된다.
        let assigns_sid = backend::assigns_session_id(&profile.command);
        let sid = if assigns_sid {
            let issued = match mode {
                SpawnMode::Resume => self.profiles.ensure_session_id(profile.id),
                SpawnMode::Fresh => self.profiles.new_session_id(profile.id),
            };
            Some(issued.ok_or_else(|| profile_vanished_mid_spawn(profile.id, "세션 id 발급"))?)
        } else {
            None
        };

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

        // ADR-0079: json 모드 claude 만 실제로 transcript 를 읽는다 — 터미널은 TUI PTY repaint 로
        //   복원되고 shell 은 대화가 없어, 그 외 backend 는 빈 Vec 을 돌려준다.
        let seed_events = match mode {
            SpawnMode::Resume => match sid {
                Some(s) => backend::resume_transcript_events(&profile.command, &cwd, s),
                None => Vec::new(),
            },
            SpawnMode::Fresh => Vec::new(),
        };

        // ADR-0191: 통로 실물과 세션에 실릴 값(backend caps·encoder·턴 분류자·우편 자격)을 backend 가
        //   **한 dispatch 로** 내준다 — 아스펙트마다 같은 switch 를 다시 타지 않는다. 이 자리는 돌려받은
        //   통로의 실제 타입을 모른다.
        //   우편 자격을 세션에 싣는 이유는 그대로다 — 프로필이 지워져도 산 세션이 그 사실을 계속 안다
        //   (`AgentManager::reads_messages` doc).
        // ★이 호출이 자식 프로세스를 띄운다 — 위 transcript 읽기보다 반드시 뒤★: 앞뒤를 바꾸면 그
        //   프로그램이 이미 도는 상태에서 그 대화 파일을 읽게 된다.
        // ADR-0185: 세션 id 를 **받아 오는** backend(codex app-server)가 그 값을 프로필에 남길 통로를 여기서
        //   건넨다.
        // ★무조건 건네는 것이 의도다 — `receives_session_id()` 같은 선언 축을 새로 만들지 않았다★:
        //   이 자리의 형제 둘(`supports_control_channel` · `accepts_mcp_config`)이 선언으로 갈리는 것은
        //   **주면 효과가 나기 때문**이다(토큰·config 파일 발급). 이 칸은 반대다 — 안 읽는 backend 에게는
        //   아무 효과도 없어서, 축을 세우면 같은 판정이 두 곳(선언 표 + impl)에 적히고 둘이 어긋날 수 있다.
        //   판정 지점은 impl 하나로 둔다.
        //   위에서 확정된 `epoch` 을 그대로 묶어, 이 spawn 이 죽은 뒤 도착한 기록이 다음 화신을 덮지 않게 한다.
        // ★`sid` 를 그대로 넘기지 않는다 — 이 칸은 **저장된 backend sid** 다(ADR-0185 의 두 축)★:
        //   `sid` 는 발급 축이 켜진 backend 에만 채워지므로, 받아 적는 쪽(codex)에서는 이어받을 값이
        //   실제로 프로필에 있어도 언제나 `None` 이다. 이어받기 요청을 **통로가** 내는 backend 는 그
        //   값으로 이어받는다.
        // ★Fresh 면 비워서 넘긴다★ — 값을 실어 보내고 backend 가 모드를 다시 보게 하면 판정이 두 곳이
        //   된다. 그 죽은 화신의 thread id 로 새 대화를 열라는 요청이 Fresh 인데, 여기서 안 비우면 그
        //   요청이 backend 마다 다르게 해석된다.
        // ★**호출자가 준 스냅샷이 아니라 명부를 읽는다**★ — `profile` 은 호출자가 뜬 사본이고, 이 칸의
        //   값은 **우리가 아니라 상대가 쓴다**(codex 가 핸드셰이크에서 준 thread id 를 기록 포트가 명부에
        //   적는다). 그래서 스냅샷 시점 뒤에 도착한 손잡이는 사본에 없다 — 그걸 읽으면 낡은 스레드로
        //   이어받는다. 발급 축 backend(claude)가 위에서 [`ProfileRegistry::ensure_session_id`] 로 명부를
        //   거치는 것과 같은 규율이고, 이 칸만 사본을 읽던 것이 **codex 쪽에서만 벌어져 있던 폭**이다.
        //   ★이 읽기가 실제로 값을 갖는 것은 [`ProfileRegistry::upsert_preserving_hierarchy`] 가
        //   `backend_session_id` 를 보존하게 된 뒤부터다(사용자 결정)★ — 그 전에는 위
        //   `register_for_spawn` 이 스냅샷으로 이 칸을 덮어써서, 명부를 읽어도 읽히는 것이 스냅샷이었다.
        // ★읽는 자리가 위 `epoch_for_spawn` **뒤**인 것이 이 읽기의 근거다★ — 표식이 새로 선 뒤부터
        //   아래 `session_id_sink` 를 건네기 전까지, **표식을 들고 오는** 기록은 전부 거절된다(앞 화신은
        //   불일치, 이 화신의 통로는 아직 없다).
        //   ★**그것이 「아무도 못 쓴다」는 뜻은 아니다 — 그렇게 적으면 거짓이다**★:
        //   [`ProfileRegistry::observe_session_id`] 의 `incarnation: None` 갈래는 대조 없이 **무조건
        //   쓴다**. 그 갈래로 들어오는 운영 호출자가 실재한다 — 데몬 조립점이 [`SessionTracker`] 에 건
        //   콜백(`engram-dashboard-daemon/src/lib.rs`)이 그것이다.
        //   그래서 「읽은 값이 얼어 있다」가 성립하는 근거는 표식 하나가 아니라 **그 관측기가 codex
        //   프로필에는 붙지 않는다**는 사실이다: `tracker.watch` 는 아래에서 `assigns_sid` 게이트 뒤에만
        //   불리고 codex 는 그 축이 꺼져 있다(ADR-0185). 그 게이트를 이어받기 축으로 바꾸는 날 이 문장이
        //   먼저 깨지므로, 그때 여기를 함께 볼 것.
        // ★`get` 이 `None` = 그 사이 프로필이 지워졌다 → **끊는다**★: 위 표식 확정을 통과한 뒤 지워진
        //   경우가 여기로 온다. `and_then` 으로 삼키면 이어받기가 **말없이 새 대화**가 되고, 그 다음
        //   `spawn_session` 이 프로필 없는 세션을 명부에 올린다. 그 둘 다 「조용히 새 대화를 만들지
        //   않는다」 규율 정면 위반이라 `?` 로 끊는다(위 sid 발급과 같은 처분).
        let resume_session_id = match mode {
            SpawnMode::Resume => {
                self.profiles
                    .get(profile.id)
                    .ok_or_else(|| profile_vanished_mid_spawn(profile.id, "이어받기 손잡이 읽기"))?
                    .backend_session_id
            }
            SpawnMode::Fresh => None,
        };
        // ★연결을 선언하는 backend 에만 배달 포트를 깐다★ — 없는 곳에 깔면 아무도 안 부르는 채널을
        //   감독자가 기다리게 되고, 그 backend 의 판정은 옛 경로 그대로여야 한다(claude·shell·stdio·
        //   codex 터미널은 여기서 `None` 이 되어 바이트 단위로 같은 길을 간다).
        // ★표식을 **여기서** 찍는다★ — 위에서 확정된 `epoch` 을 포트에 묶어, 이 화신의 결말이 다음
        //   화신에게 적용되는 일이 없게 한다(`link_verdict_sink` 의 doc 이 그 사유의 정본).
        let (link_sink, link_rx) = if backend::declares_link(&profile.command) {
            let (tx, rx) = std::sync::mpsc::channel();
            (Some(link_verdict_sink(tx, epoch)), Some(rx))
        } else {
            (None, None)
        };
        let parts = backend::open_spawn(
            &profile.command,
            &spec,
            DEFAULT_COLS,
            DEFAULT_ROWS,
            Some(session_id_sink(self.profiles.clone(), profile.id, epoch)),
            resume_session_id,
            link_sink,
        )?;

        let (session, child_pid) =
            self.spawn_session(profile.id, spec, parts, epoch, seed_events)?;

        if let Some(g) = provision_guard.as_mut() {
            g.disarm();
        }

        // sid drift 관측 부착(best-effort). 관측기를 만드는 것도 "만들 게 없다"고 답하는 것도 backend
        //   몫이라(ADR-0004) 여기서는 그 프로그램이 무엇을 읽는지 모른다.
        // ★게이트가 **발급 축**인 이유(ADR-0185)★: 관측기는 우리가 건넨 sid 를 **기준값**(`expected_sid`)
        //   으로 받아 그것과 달라진 것을 관측한다 — 발급하지 않은 세션에는 그 기준값이 아예 없다.
        //   이어받기 축으로 갈면 기준값 없는 세션에 관측기가 붙는다.
        if let (Some(s), Some(pid)) = (sid, child_pid) {
            if assigns_sid {
                if let Some(source) =
                    backend::session_id_source(&profile.command, profile.id, pid, s)
                {
                    self.tracker.watch(profile.id, source);
                }
            }
        }

        tracing::info!(agent = %profile.id, epoch, ?mode, "에이전트 spawn");

        let info = self.agent_info(&session);
        self.status_sink.agent_list_updated(self.list_agents());
        // ★배달함과 예약을 **한 벌로** 넘긴다★ — 결말을 기다리는 호출자가 그 사이 예약을 놓지 않게 하는
        //   것이 [`LinkWatch`] 의 존재 이유다. 연결 축이 없으면 `None` 이고, 그때 예약은 예전 그대로
        //   이 함수와 함께 끝난다(`reservation` 이 여기서 drop 된다).
        let watch = match link_rx {
            Some(rx) => Some(LinkWatch {
                rx,
                _reservation: reservation,
            }),
            None => None,
        };
        Ok((SpawnOutcome::Started(info), watch))
    }

    /// ★수동 활성화 진입점 — 이어받기(resume) 전용, fresh-fallback 폐지(ADR-0082)★.
    /// SpawnProfile 핸들러가 `spawn_agent` 대신 이걸 부른다. 세 갈래로 나뉜다:
    ///
    /// 1. **이미 실행 중(재활성화 가드)** — 같은 id 세션이 살아 있으면 **아무것도 죽이거나
    ///    재spawn 하지 않고** 그 세션의 AgentInfo 를 그대로 돌려준다(무해한 "이미 실행 중" 신호,
    ///    epoch 불변). ★이게 a4aac1a 회귀의 핵심 수정★: 예전엔 이 경로가 `spawn_agent` 이중-spawn
    ///    가드가 내던 "already running" **Err** 를 만나 `resume_with_fresh_fallback` 이 그걸 "resume
    ///    실패"로 오인 → `fallback_fresh` 가 **멀쩡히 돌던 산 에이전트를 kill** → 새 화신 표식 → 빈 fresh
    ///    로 교체(유저 실측 회귀). 지금은 두 겹으로 막힌다: 여기 선제 조회가 먼저 걸러 산 에이전트를
    ///    놔두고, 그걸 지나쳐도 그 가드는 이제 Err 가 아니라 `SpawnOutcome::Moot` 을 낸다(오인할 오류
    ///    자체가 없다).
    /// 2. **Fresh(진짜 신규 — 세션 없음)** — `spawn_fresh_settled` 위임. 이건 실패-fallback 이 아니라
    ///    정상 신규 생성이다(ADR-0076 "Fresh=새 sid" 유효).
    /// 3. **Resume** — `resume_no_fallback` 로 이어받기만 시도하고, 그 Failed 결말을 Err 로 노출한다.
    ///
    /// ★blocking★: 호출이 얼마나 블록되나는 **모드가 아니라 통로의 연결 축**이 정한다.
    ///   - 연결을 선언하지 않는 통로(claude·shell·stdio·codex 터미널): Resume 은 최대
    ///     EARLY_EXIT_WINDOW(현 3s)만큼 폴링하고, Fresh 는 폴링 없이 즉시 반환한다(옛 성질 그대로).
    ///   - 연결을 선언하는 통로(codex app-server): **Fresh 도 Resume 도** 연결의 결말이 날 때까지
    ///     기다린다 — 다만 그것은 창이 아니라 **신호**라(ADR-0201) 정상 왕복이면 즉시 돌아온다
    ///     (실측 약 130ms). 상한은 [`LINK_RESOLUTION_BACKSTOP`] 이고 그것은 판정 창이 아니라 포기다.
    ///   재활성화 가드와 아래 in-flight 가드는 어느 축에서도 폴링 없이 즉시 반환한다.
    /// ★★그래서 이 동사는 **blocking 풀에서만 불러야 한다 — 호출자의 의무다**★★: 최악은 백스톱(15s) +
    ///   실패 판정 뒤 teardown 의 `session.kill(5s)` 이고 그 사이 **yield 지점이 없다.** async 워커에서
    ///   직접 부르면 그 워커가 통째로 묶이고, 막힌 활성화 N 건이 워커 N 개를 가져가 다른 연결의 명령까지
    ///   함께 멈춘다. 오늘 운영 호출자 넷이 전부 그 계약을 지킨다 — 명령 버스 둘은 `blocking_handler`
    ///   뒤(`crate::commands::make_table`), WS 둘은 `tokio::task::spawn_blocking` 뒤
    ///   (`connection_core` 의 `Spawn`·`SpawnProfile`). ★한때 이 자리에 「그 연결의 응답만 지연되고
    ///   다른 세션에는 영향 없다」로 적혀 있었는데, 그것은 **호출자가 위 계약을 지킬 때만** 참이고
    ///   실제로 WS 둘이 안 지키고 있었다★ — 성질이 아니라 계약으로 적는다.
    ///   ★실패는 이 상한보다 빨리 돌아올 수 있다★ — 진단 스트림이 먼저 말하면 그 자리에서 끊는다.
    // ADR-0082
    // ADR-0076
    // ADR-0201
    pub fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<AgentInfo, PtyError> {
        // ★★「떠 있다」고 답하기 **전에** 「아직 결말이 안 난 활성화」를 본다 — 순서를 되돌리지 마라★★:
        //   연결을 선언하는 통로에서는 세션이 명부에 오른 뒤에도 그 화신이 입력을 받을 수 있는지가
        //   아직 안 정해져 있고([`LinkWatch`]), 그 구간을 여는 요청이 예약을 들고 있다. 아래 조회를
        //   먼저 두면 그 구간의 두 번째 활성화가 `Ok`(도는 중)를 받는데, 첫 요청이 곧 실패로 판정해
        //   그 화신을 거두면 **두 호출자가 서로 다른 사실을 들고 갈린다** — 뒤엣것은 자기 밑에서 죽을
        //   에이전트를 산 것으로 보고한다.
        // ★`Err` 가 정직한 답인 이유★: 이 요청은 아무것도 하지 않았고 돌려줄 산 세션도 없다. 같은 문구를
        //   아래 Fresh 갈래가 이미 쓰고 있다(승자가 아직 명부에 올리기 전인 경우) — 그 답을 **연결이
        //   설 때까지로 늘린 것**이 여기다.
        // ★이 가드는 창을 **줄이지만 닫지는 않는다**★ — 이 검사와 `spawn_agent_watching_link` 의 예약
        //   취득 사이에 여전히 명령 몇 개 폭의 창이 있다(ADR-0082 「열린 항목」 ③ 의 선재 레이스).
        //   닫은 것은 핸드셰이크 한 왕복만큼 벌어져 있던 **긴** 창이다.
        if self.activation_in_flight(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: 다른 요청의 활성화가 아직 결말을 못 냈다 — 이 요청은 할 일이 없다"
            );
            return Err(PtyError::SpawnFailed(format!(
                "another request is already activating agent {}; this one did nothing",
                profile.id
            )));
        }

        if let Ok(session) = self.get_session(profile.id) {
            tracing::info!(
                agent = %profile.id,
                "activate_profile: 이미 실행 중 — 재활성화 무시(산 에이전트 보존, ADR-0082)"
            );
            // ★이 갈래는 「마지막 실패」에 아무것도 쓰지 않는다★: 활성화를 시도하지 않았으므로 기록할
            //   실패도, 지울 근거도 없다 — 아래 `spawn_agent` 의 moot 갈래와 같은 규율이다(ADR-0172).
            return Ok(self.agent_info(&session));
        }

        if mode == SpawnMode::Fresh {
            // ★기록은 이 안에서 한다 — 여기서 또 쓰면 지움 지점이 둘이 된다★(연결 결말을 본 자리만이
            //   무엇이 일어났는지 안다. Resume 갈래가 `resume_no_fallback` 에 맡기는 것과 같은 규율).
            let outcome = self.spawn_fresh_settled(profile);
            // moot 이면 남이 띄운(띄우는 중인) 세션을 돌려준다. 아직 명부에 없으면 조회로 한 번 더 본다.
            // ★그마저 없을 때의 문구는 "없는 에이전트" 가 아니다★: 프로필은 실재하고 지금 **다른 요청이
            //   띄우는 중**이라 우리가 돌려줄 세션이 없을 뿐이다. `NotFound` 를 그대로 흘리면 원인을
            //   잘못 지목한 문구("agent not found")가 호출자·LLM 에게 간다.
            // ADR-0172
            return match outcome?.into_info() {
                Some(info) => Ok(info),
                None => self.agent_info_by_id(profile.id).map_err(|e| match e {
                    PtyError::NotFound(id) => PtyError::SpawnFailed(format!(
                        "another request is already starting agent {id}; this one did nothing"
                    )),
                    other => other,
                }),
            };
        }

        let (outcome, spawned) = self.resume_no_fallback(profile);
        match outcome {
            // 성립·실패 양쪽 다 `resume_no_fallback` 안에서 이미 기록됐다(그 자리가 무엇이 일어났는지
            //   아는 유일한 곳). 여기서 또 쓰면 지움 지점이 둘이 된다.
            //
            // ★조회 실패로 성공을 뒤집지 않는다★: 조기종료 창을 넘긴 순간 우리는 성립으로 판정하고 기록을
            //   **지웠다**. 그 직후 자식이 죽어 reaper 가 거둬 가면 조회는 실패하는데, 그때 Err 를 내면
            //   "실패했다고 답하면서 기록은 지운" 모순이 된다. 그래서 spawn 시점 정보로 떨어진다.
            // ★단 그 정보를 **산 것으로** 돌려주지는 않는다★: spawn 시점 스냅샷의 status 는 `Running` 이라
            //   그대로 내면 호출자가 시체를 산 에이전트로 보고한다(`state_of` 가 "live" 로 번역하고, WS 는
            //   그 status 로 `Spawned` 를 낸다 — 그러면 다음 동사가 시체에게 편지를 쓴다). 조회가 놓쳤다는
            //   것 자체가 "그 사이 수거됐다" 는 관측이므로 종점 상태로 낮춰 싣는다. 코드는 모른다(`None`).
            // ADR-0172
            // ★`Started` 를 `Resumed` 와 **같이** 처리한다 — 둘 다 「떴다」이고 갈리는 것은 보고 어휘다★:
            //   앞엣것은 이어받을 손잡이가 없어 새 대화를 연 경우다(`resume_no_fallback` 의 그 판정).
            //   여기서 `Err` 로 내리면 정상적으로 뜬 에이전트가 실패로 보고된다.
            RestoreOutcome::Resumed | RestoreOutcome::Started => {
                match self.agent_info_by_id(profile.id) {
                    Ok(info) => Ok(info),
                    Err(e) => match spawned {
                        Some(mut info) => {
                            info.status = AgentStatus::Exited { code: None };
                            Ok(info)
                        }
                        None => Err(e),
                    },
                }
            }
            RestoreOutcome::Failed { reason } => Err(PtyError::SpawnFailed(reason)),
            // resumable 프로필로만 진입하므로 나머지 결말은 도달 불가(방어적 Err).
            // ★방어 갈래는 상태를 건드리지 않는다★: 무슨 일이 있었는지 모르는 자리라, 실패를 단정해 쓰면
            //   방금 뜬 에이전트에 도장을 찍을 수 있다(그것도 epoch 가드 없이).
            other => Err(PtyError::SpawnFailed(format!(
                "activate_profile: 예상 밖 결말 {other:?}"
            ))),
        }
    }

    /// ★「마지막 실패」를 쓰는 **유일한 지점**(ADR-0172 결정 4 — 개정판)★. `None` = 활성화가 성립했다
    /// → 지운다. `Some(kind)` = 그 자리에서 기록한다.
    ///
    /// ★지우는 조건이 「대화 왕복 성립」에서 「활성화 성공」으로 바뀐 이유 — 되돌리지 마라★:
    ///   왕복 성립은 **턴 종료 신호**로만 관측되는데 그 신호는 구조화(stream-json) 출력에서만 나온다
    ///   (터미널·shell 은 분류자가 침묵). 반대로 실패 **분류**는 콘솔 바이트에서만 나온다(구조화 세션은
    ///   진단을 stderr 로 흘린다). 두 반쪽이 출력 형식으로 갈려 서로 배타적이라, 왕복 기준으로는
    ///   터미널 에이전트의 기록이 **영영 안 지워진다**. 게다가 그 신호는 epoch 게이트가 없고
    ///   (`StatusSink::turn_ended` — "잉여는 무해한 no-op") 논블록 계약이라, 거기서 프로필 lock 을 잡는
    ///   것 자체가 계약 위반이었다.
    /// ★성공을 지움 근거로 삼는 것이 정직한 이유★: `--resume` 가 조기종료 창을 넘겼으면 이어받을 대화가
    ///   **실재했다**는 뜻이고, fresh 가 떴으면 이제 하나 생긴다. 그 뒤 한 마디도 없이 닫으면 **다음
    ///   활성화가 다시 실패해 다시 기록된다** — 그게 「미리 감지하지 않고 실패한 자리에서 기록한다」다.
    /// ★기록과 같은 스레드에서 부른다★: 활성화를 집행하는 그 스레드다(출력 pump 아님).
    /// ★`incarnation` 은 지각-쓰기 가드다★ — 계약은 `ProfileRegistry::set_last_failure` 가 갖는다.
    ///   화신이 만들어진 결말(성공 · 조기종료)은 그 epoch 을 실어 보내고, 화신 전에 끝난 실패만 `None` 이다.
    // ADR-0172
    fn note_activation_result(
        &self,
        id: AgentId,
        incarnation: Option<u32>,
        failure: Option<AgentFailureKind>,
    ) {
        self.profiles.set_last_failure(id, incarnation, failure);
    }

    /// 이 항목의 활성화가 **아직 결말을 못 낸 채 진행 중**인가.
    ///
    /// ★예약 집합을 그대로 읽는 것이 전부다 — 두 번째 상태를 만들지 않았다★: 그 집합의 수명이 이미
    ///   정확히 「누군가 이 항목을 띄우는 중」이고, 연결을 선언하는 통로에서는 [`LinkWatch`] 가 그
    ///   수명을 **연결의 결말까지** 늘려 둔다. 별도 「establishing」 표를 만들면 세울 지점과 지울 지점이
    ///   새로 둘 생기고, 통로가 자기 상태를 칸에 담아 두고 감독자가 들여다보던 옛 모양으로 되돌아간다
    ///   (그 모양이 이 라운드가 걷어낸 결함 넷의 뿌리다).
    /// ★락을 잡는 구간은 `contains` 하나다★ — 이 Mutex 보유 중 sessions/profiles 를 잡지 않는다
    ///   (ADR-0006 · `spawning` 필드 doc).
    fn activation_in_flight(&self, id: AgentId) -> bool {
        self.spawning
            .lock()
            .expect("spawning set poisoned")
            .contains(&id)
    }

    /// **spawn 한 번으로 결말이 나는** 갈래의 기록 — 띄웠으면 그 화신의 epoch 으로 지우고, 실패면
    /// 기록하고, **할 일이 없었으면 아무것도 쓰지 않는다**.
    ///
    /// ★「Fresh 의 기록」이 아니다 — 호출자는 이제 [`AgentManager::spawn_fresh_settled`] 하나뿐이고 그
    ///   안에서도 **연결 축이 없는 통로**에만 쓰인다★: 연결을 선언하는 통로는 spawn 이 `Started` 를
    ///   돌려준 시점에 아직 결말이 안 났으므로(핸드셰이크가 시작도 안 했다) 여기서 지우면 그것이 곧
    ///   ADR-0202 가 막으려는 증거 파괴다. 그쪽 갈래는 판정을 본 뒤 자기 자리에서 쓴다.
    ///
    /// ★moot 이 아무것도 안 쓰는 것은 예외가 아니라 정의다★: 이 호출은 아무것도 하지 않았으므로 기록할
    ///   실패도 없고 지울 근거도 없다. `activate_profile` 의 선제 "이미 실행 중" 갈래가 쓰지 않는 것과
    ///   같은 이유다.
    /// resume 은 이걸 쓰지 않는다 — spawn 이 Ok 여도 조기종료 창을 넘겨야 성립이라, 결말을 아는 자리가
    /// `resume_no_fallback` 이다.
    // ADR-0172
    fn note_spawn_result(&self, id: AgentId, result: &Result<SpawnOutcome, PtyError>) {
        match result {
            Ok(SpawnOutcome::Started(info)) => {
                self.note_activation_result(id, Some(info.epoch), None)
            }
            Ok(SpawnOutcome::Moot(_)) => {}
            // 화신이 없으므로 비교할 세대가 없다(`set_last_failure` 계약의 `None` 갈래).
            Err(_) => self.note_activation_result(id, None, Some(AgentFailureKind::SpawnFailed)),
        }
    }

    /// Fresh 활성화 — ★프로세스가 떴다는 것만으로 성공을 말하지 않는다★.
    ///
    /// ★왜 이 함수가 생겼나(회귀 방지 — 지우면 그 구멍이 그대로 돌아온다)★: `spawn_agent` 은 **자식
    ///   프로세스가 뜬 순간** `Started` 를 돌려주고 배달함을 버린다. 연결을 선언하는 통로에서는 그
    ///   시점에 핸드셰이크가 아직 시작도 안 했다 — 그래서 Fresh 로 띄운 codex 가 `thread/start` 를
    ///   거절당해도 **아무도 그 결말을 보지 않았고**, 두 가지가 한꺼번에 일어났다:
    ///   ① 상대가 stdout 을 붙든 채 남으면 리더가 EOF 를 못 봐 [`OutputCore::finish`] 가 영영 안 돌고
    ///      그 메시지의 단독 소비자인 reaper 도 안 움직인다 — 화면엔 Running 인데 입력은 전부 거절되는
    ///      **굳음**이 돌아온다(ADR-0200 이 이어받기 쪽에서 닫은 바로 그 모양).
    ///   ② `note_spawn_result` 가 그 `Started` 를 성공으로 읽고 **「마지막 실패」를 지웠다** — 거절당한
    ///      활성화가 성공으로 보고되면서 앞선 실패 증거까지 함께 파괴했다(ADR-0202 가 「아직 참이 아니다」로
    ///      적어 둔 그 구멍).
    ///   Fresh 는 새 codex 에이전트의 **일상 경로**이지 변두리가 아니다.
    ///
    /// ★그래서 「성공」의 뜻을 Resume 과 **같은 것**으로 둔다(ADR-0201)★: 성공 = 에이전트가 **입력을
    ///   받을 준비가 됐다고 신호했다**. 프로세스가 살아 있다는 것은 그 질문의 답이 아니다. 새 대화든
    ///   이어받기든 상대에게 묻는 것은 같으므로(`thread/start` vs `thread/resume` — 통로 안쪽의 갈림),
    ///   판정 어휘를 갈라 둘 이유가 없다.
    /// ★성공이 「마지막 실패」를 **지우지 않는다**(ADR-0202)★ — 준비됐다는 관측은 그 뒤의 일을 말해
    ///   주지 않고, 지움은 되돌릴 수 없다. 자동 지움은 어느 갈래에서도 안 한다.
    /// ★연결 축이 없는 통로는 **한 줄도 바뀌지 않는다**★ — 그쪽은 아래 let-else 로 빠져 옛
    ///   `note_spawn_result` 경로(띄웠으면 지움)를 그대로 탄다. claude 의 Fresh 는 지금 깨져 있지 않고,
    ///   ADR-0201 이 그 경로를 이번 범위 밖으로 못박았다.
    /// ★실패 종류는 분류하지 않고 [`AgentFailureKind::Other`] 로 둔다★ — `resume_failure_kind` 는
    ///   **이어받기** 어휘(「이어받을 대화가 없다」)를 내는데, 새 대화를 여는 이 갈래에서 그 문구는
    ///   거짓이다(이어받으려 한 적이 없다). 없는 어휘를 지어내는 대신 「그 밖」으로 둔다 — 그쪽은
    ///   재시도 가능이라 화면이 항목을 막지도 않는다(`failureKinds.ts`).
    // ADR-0201
    // ADR-0202
    fn spawn_fresh_settled(&self, profile: &AgentProfile) -> Result<SpawnOutcome, PtyError> {
        let (result, watch) = match self.spawn_agent_watching_link(profile, SpawnMode::Fresh) {
            Ok((outcome, watch)) => (Ok(outcome), watch),
            Err(e) => (Err(e), None),
        };

        // 기다릴 결말이 없는 셋이 여기서 빠진다 — 연결 축이 없는 통로 · 아무것도 안 만든 요청(moot) ·
        // 프로세스조차 못 띄운 실패. 셋 다 옛 기록 규율 그대로다(`note_spawn_result`).
        let (Some(watch), Ok(SpawnOutcome::Started(info))) = (watch.as_ref(), &result) else {
            self.note_spawn_result(profile.id, &result);
            return result;
        };
        // ★표식을 결말 **밖으로** 들고 나간다★ — 아래 정리·기록은 판정만큼 늦게 일어나고, 그 사이 이
        //   화신이 수거되고 다음 화신이 떴을 수 있다. 값으로 들고 가야 둘 다 이 화신에만 닿는다
        //   (ADR-0163/0164 — 비교는 일치/불일치뿐).
        let incarnation = info.epoch;

        match self.link_activation_verdict(
            profile.id,
            incarnation,
            &watch.rx,
            LINK_RESOLUTION_BACKSTOP,
        ) {
            // ★상대가 준비됐다고 신호했다 = 성립★. 기록은 건드리지 않는다(위 doc · ADR-0202).
            //
            // ★★스냅샷을 **다시 뜬다** — `result` 를 그대로 돌려주지 마라★★: 그 안의 `info` 는 자식이
            //   뜬 **직후**에 조립됐고 status 가 `Running` 이다. 예전에는 그 값이 0ms 된 것이라 그대로
            //   내도 무해했지만, 지금 이 자리는 연결의 결말을 기다린 **뒤**라 최대 백스톱만큼 낡았다.
            //   그 사이에 자식이 죽었으면 호출자는 시체를 산 에이전트로 보고받고(WS `Spawned` 가 그
            //   status 를 그대로 나른다) **다음 동사가 시체에게 편지를 쓴다.**
            // ★조회가 실패하면 성공을 뒤집는 것이 아니라 상태만 낮춘다★ — 조회가 놓쳤다는 것 자체가
            //   「그 사이 수거됐다」는 관측이다. 활성화는 성립했으므로 `Err` 로 바꾸지 않는다.
            //   코드는 모른다(`None`). `resume_no_fallback` 의 형제 갈래를 `activate_profile` 이
            //   처리하는 모양과 **같은 규율**이고, 그쪽 주석이 이 인과의 정본이다.
            EarlyVerdict::Ready => match self.agent_info_by_id(profile.id) {
                Ok(fresh) => Ok(SpawnOutcome::Started(fresh)),
                Err(_) => {
                    let mut gone = info.clone();
                    gone.status = AgentStatus::Exited { code: None };
                    Ok(SpawnOutcome::Started(gone))
                }
            },
            // ★정리를 **먼저**, 기록을 나중에★ — `resume_no_fallback` 의 같은 갈래와 같은 순서이고 같은
            //   사유다: 기록은 프로필 뮤텍스를 잡는데 그 뮤텍스가 붙들려 있으면 순서가 뒤집힌 쪽은
            //   **자식을 영영 안 죽인다**. 백스톱은 바로 그 정체를 위해 있다.
            EarlyVerdict::LinkFailed { reason } => {
                self.tear_down_failed_activation(profile.id, incarnation);
                self.note_activation_result(
                    profile.id,
                    Some(incarnation),
                    Some(AgentFailureKind::Other),
                );
                tracing::warn!(
                    agent = %profile.id,
                    epoch = incarnation,
                    %reason,
                    "새 대화의 연결이 서지 못했다 → 활성화 실패 — LLM 에스컬레이션 대상"
                );
                Err(PtyError::SpawnFailed(format!(
                    "새 대화 연결 실패: {reason}"
                )))
            }
            // 판정이 끝나기 전에 프로세스가 죽었다 — 자식이 이미 종점이라 정리할 것이 없다(ADR-0082
            // 「아무것도 죽지마」: 관측된 시체는 그대로 둔다).
            // ★사용자가 끊은 것은 활성화 실패가 아니다★ — 기록하지 않는다(`resume_no_fallback` 의 같은 규율).
            EarlyVerdict::Terminal { status, evidence } => {
                if matches!(status, AgentStatus::Killed) {
                    tracing::info!(
                        agent = %profile.id,
                        epoch = incarnation,
                        "새 대화가 서는 중에 사용자가 종료했다 — 활성화 실패로 기록하지 않는다"
                    );
                } else {
                    self.note_activation_result(
                        profile.id,
                        Some(incarnation),
                        Some(AgentFailureKind::Other),
                    );
                    tracing::warn!(
                        agent = %profile.id,
                        epoch = incarnation,
                        ?status,
                        %evidence,
                        "새 대화가 서기 전에 종료했다 → 활성화 실패 — LLM 에스컬레이션 대상"
                    );
                }
                // ★꼬리를 **오류 문구에 싣지 않는다**★ — 최대 4 KiB 짜리 콘솔 꼬리가 명령 응답으로
                //   그대로 나간다. 위 로그가 그것을 나르고, 문구는 `resume_no_fallback` 의 형제와 같은
                //   모양(상태 하나)으로 둔다.
                Err(PtyError::SpawnFailed(format!(
                    "새 대화가 서기 전에 종료({status:?})"
                )))
            }
            // ★도달 불가 — [`AgentManager::link_activation_verdict`] 는 이 둘을 내지 않는다★.
            //   방어 갈래는 상태를 건드리지 않는다: 무슨 일이 있었는지 모르는 자리라, 실패를 단정해 쓰면
            //   방금 뜬 에이전트에 도장을 찍을 수 있다.
            other => Err(PtyError::SpawnFailed(format!(
                "spawn_fresh_settled: 예상 밖 결말 {other:?}"
            ))),
        }
    }

    /// ★통로를 여기서 만들지 않는다(ADR-0191)★ — 이미 만들어진 것을 `parts` 로 받고, 이 함수는 그
    ///   실제 타입을 모른다. 호출자가 넘기기 전에 **자식 프로세스는 이미 떠 있다**.
    fn spawn_session(
        &self,
        id: AgentId,
        spec: CommandSpec,
        parts: backend::SpawnParts,
        epoch: u32,
        seed_events: Vec<OutputEvent>,
    ) -> Result<(Arc<AgentSession>, Option<u32>), PtyError> {
        let backend::SpawnParts {
            transport,
            child_pid,
            backend_caps,
            encoder,
            turn_classifier,
            reads_messages,
        } = parts;

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
        // ADR-0185: 이어받기 축 — 저장된 sid 를 **누가 발급했는지는 묻지 않는다**(그 축은 spawn 시점의
        //   `assigns_session_id` 몫). 판정 규칙은 다른 활성화 입구들과 한 몸이라 dispatch 가 갖는다.
        let resumable = backend::can_resume_profile(profile);

        if !resumable {
            // ADR-0172: 부팅 복원도 같은 규율 — 띄웠으면 지우고 실패하면 그 자리에서 기록한다.
            // ★수동 활성화와 **같은 동사**를 쓴다(ADR-0201/0202)★: 이어받을 손잡이가 없는 codex 프로필은
            //   부팅 복원에서도 이 갈래로 오고, 거기서 `thread/start` 가 거절되면 아무도 그 결말을 보지
            //   않는 옛 구멍이 **프로필 수만큼** 한꺼번에 선다. 입구마다 판정을 갈라 두면 그중 하나가
            //   반드시 뒤처진다.
            let outcome = self.spawn_fresh_settled(profile);
            return match outcome {
                // moot(이미 떠 있음)도 결과적으로 "그 항목은 떠 있다" 라 같은 보고로 접는다 — 복원 보고
                //   어휘에 「할 일 없었음」 칸이 없다(그 칸을 만드는 것은 wire 변경이라 별건).
                Ok(_) => RestoreOutcome::Started,
                Err(e) => RestoreOutcome::Failed {
                    reason: e.to_string(),
                },
            };
        }

        self.resume_no_fallback(profile).0
    }

    /// ★resume 전용 공용 규율(ADR-0082 — 부팅복원·수동활성화 공유, fresh-fallback 폐지)★.
    /// 전제: 호출 시점에 이 프로필은 resumable(claude + sid 존재)이라고 이미 판정됐다.
    ///
    /// resume 을 시도하고, spawn 실패거나 EARLY_EXIT_WINDOW 안에 **비정상 종료하거나 진단 스트림이
    /// 실패를 말하면**(빈/미대화/손상 세션이면 claude 가 "No conversation found ..." 를 stderr 로 낸다)
    /// **새 대화(fresh)를 자동으로 만들지 않고** 종점(시체)으로 직행한다 — 사유를 로그로 남겨
    /// LLM 이 읽고 에스컬레이션한다.
    /// ★그 종점은 `AgentStatus::Failed` 가 아니다★: 상태는 `TerminalReason` 하나로만 갈리고
    ///   (`OutputCore::finish`) 이 경로엔 상태를 쓰는 줄이 없다 — resume child 가 스스로 죽으므로
    ///   `Exited{code}`(대개 code≠0)로 간다. 아래 `RestoreOutcome::Failed` 는 **다른 축**(활성화 결과)이다.
    /// ★"즉사" 가 아니다(실측 2026-08-23 정정)★: claude 는 그 진단을 내고도 종료 훅이 도는 동안 몇 초
    ///   더 산다. 그래서 판정은 죽음이 아니라 증거로도 선다(`EarlyVerdict::Diagnosed`).
    /// ★아무것도 kill·재spawn 하지 않는다★: resume child 는 자기 pump 가 EOF→finish 하고, reaper 가
    ///   그 세션을 맵에서 수거하며 프로필을 `auto_restore=false`(KeepDisableAutoRestore)로 내려
    ///   트리에 **관측된 종점 그대로** 시체로 남긴다(대개 `Exited{code≠0}` — profile 은 지워지지 않음:
    ///   exit≠0/불명은 삭제 대상이 아님).
    ///   이 헬퍼는 종료를 관측만 하고 어떤 파괴 동작도 하지 않는다(옛 fallback_fresh 의 remove_session·
    ///   화신 표식 재발급·respawn 을 전부 걷어냈다 — ADR-0082 사용자 결정: "아무것도 죽지마, 새로 만들지마").
    /// 이 로직을 restore_one(부팅 복원)과 activate_profile(수동 활성화)이 **똑같이** 재사용한다.
    ///
    /// 반환의 둘째 칸 = 이 시도가 **만들어 낸** 화신(있으면). `activate_profile` 이 성공 판정 뒤 조회가
    /// 실패했을 때 성공을 뒤집지 않으려고 쓴다(그쪽 주석이 그 인과의 정본).
    // ADR-0082
    // ADR-0172
    fn resume_no_fallback(&self, profile: &AgentProfile) -> (RestoreOutcome, Option<AgentInfo>) {
        // ★이 시도가 **실제로 이어받는가**를 spawn 전에 확정한다(사용자 결정)★ — 「새 대화를 열어 놓고
        //   이어받았다고 보고하지 않는다」.
        //   조건 둘이 다 참일 때만 「이어받기인 척하는 새 대화」가 된다: ① 이어받기 요청을 **통로가**
        //   내는 backend 다(= 손잡이를 우리가 발급하지 않는다 — 발급하는 쪽은 `ensure_session_id` 가
        //   반드시 값을 만들어 주므로 이 창이 없다) ② 그런데 명부에 손잡이가 없다.
        //   ★그 조합에 실제로 들어오는 것은 WS `SpawnProfile` 의 `resume: true` 명시 요청이다★ — 그
        //   플래그는 저장된 세션이 없어도 Resume 으로 남기므로(그 자리 주석), codex 에서는 통로가
        //   `thread/start` 로 **새 스레드**를 연다. claude 는 ①에서 걸러져 옛 경로 그대로다.
        //   ★동작을 바꾸지 않는다 — 바꾸는 것은 **보고**뿐이다★: 아무것도 덮어쓰지 않고(덮어쓸 손잡이가
        //   애초에 없다) 새 대화는 그대로 뜬다. 다만 그 결말을 `Resumed` 가 아니라 `Started` 로 낸다.
        let opens_a_new_conversation = !backend::assigns_session_id(&profile.command)
            && backend::can_resume_stored_session(&profile.command)
            && self
                .profiles
                .get(profile.id)
                .and_then(|p| p.backend_session_id)
                .is_none();
        if opens_a_new_conversation {
            tracing::warn!(
                agent = %profile.id,
                "이어받기 요청인데 저장된 손잡이가 없다 — 새 대화를 연다(결말은 `Resumed` 가 아니라 `Started` 로 보고한다)"
            );
        }

        let (outcome, watch) = match self.spawn_agent_watching_link(profile, SpawnMode::Resume) {
            Err(e) => {
                let reason = format!("resume spawn 실패: {e}");
                // ADR-0172: 실패는 시도한 자리에서 기록한다 — 이 기록이 ADR-0082 가 요구한 "원인을 남겨
                //   제어 LLM 이 읽는다" 의 화면·API 쪽 실물이다(로그는 사람만 읽는다).
                self.note_activation_result(profile.id, None, Some(AgentFailureKind::SpawnFailed));
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    "ADR-0082: resume 실패 → 종점(시체), fresh-fallback 없음"
                );
                return (RestoreOutcome::Failed { reason }, None);
            }
            Ok(pair) => pair,
        };
        let spawned = match outcome {
            // ★할 일이 없었다 — 조기종료 창을 볼 이유도 없다★: 남이 띄운(띄우는 중인) 세션이라 우리가
            //   관측한 것이 없고, 그래서 기록도 지움도 하지 않는다(`note_spawn_result` 와 같은 규율).
            //   보고는 「떠 있다」로 접는다 — 복원 어휘에 「할 일 없었음」 칸이 없다.
            // ADR-0172
            SpawnOutcome::Moot(info) => {
                tracing::info!(
                    agent = %profile.id,
                    "resume: 이미 떠 있거나 뜨는 중 — 이 요청은 할 일이 없다(moot)"
                );
                return (RestoreOutcome::Resumed, info);
            }
            SpawnOutcome::Started(info) => info,
        };

        // ★fable M-1★: 성공한 claude resume은 TUI라 윈도 안에 종료하지 않는다.
        // 따라서 윈도 내 terminal 진입은 code와 무관하게 resume 실패 신호다
        // (code==0 조기 종료를 Resumed로 오판하면 빈 화면을 "복원 성공"으로 오보).
        //
        // ★`spawned.epoch` 을 창 **밖으로** 들고 나간다(지각-쓰기 가드)★: 아래 두 쓰기는 최대
        //   EARLY_EXIT_WINDOW 뒤에 일어나고 그때 예약은 이미 풀려 있다. 그 사이 다른 연결이 같은
        //   프로필을 성공적으로 활성화하면 epoch 이 올라가므로, 이 값을 실어 보내면 옛 관측이 새 화신을
        //   덮지 못한다(`set_last_failure` 계약).
        // ADR-0172
        // ★판정 경로가 **통로의 축으로** 갈린다★: 배달 채널이 있으면 결말이 오기를 기다리고, 없으면
        //   옛 창 판정을 그대로 탄다. 둘을 한 루프에 섞지 않는 이유 = `link_activation_verdict` 의 doc.
        let verdict = match watch.as_ref() {
            Some(w) => self.link_activation_verdict(
                profile.id,
                spawned.epoch,
                &w.rx,
                LINK_RESOLUTION_BACKSTOP,
            ),
            None => self.early_activation_verdict(
                profile.id,
                &profile.command,
                EARLY_EXIT_WINDOW,
                LINK_RESOLUTION_BACKSTOP,
            ),
        };
        match verdict {
            EarlyVerdict::Terminal { status, evidence } => {
                let reason = format!("resume 조기 종료({status:?})");
                // ★사용자가 끊은 것은 활성화 실패가 아니다 — 기록하지 않는다★: 창 안에서 kill 이 오면
                //   (트리 종료·`agent.kill`·LLM 의 활성화→종료) 그건 우리가 관측할 실패가 아니라 명시적
                //   개입이다. 기록하면 트리에 「이어받은 직후 종료됐습니다」가 남아, 사용자가 방금 스스로
                //   끈 항목이 고장 난 것처럼 보인다. 활성화 결과는 여전히 Failed 다(에이전트가 안 떠 있다).
                //   지우지도 않는다 — 이 시도에 대해 성립을 주장할 근거도 없다.
                // ADR-0172
                if matches!(status, AgentStatus::Killed) {
                    tracing::info!(
                        agent = %profile.id,
                        %reason,
                        "resume 창 안에서 사용자 종료 — 활성화 실패로 기록하지 않는다"
                    );
                } else {
                    // 증거로 종류를 알아보지 못하면 맥락 기본값 — 「이어받기 직후 조기 종료」(재시도 가능).
                    let kind = backend::resume_failure_kind(&profile.command, &evidence)
                        .unwrap_or(AgentFailureKind::EarlyExitAfterResume);
                    self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                    tracing::warn!(
                        agent = %profile.id,
                        %reason,
                        ?kind,
                        "ADR-0082: resume 조기종료 → 종점(시체), fresh-fallback 없음 — LLM 에스컬레이션 대상"
                    );
                }
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★아직 살아 있는데 실패로 접는다 — 그리고 **아무것도 죽이지 않는다**★: claude 는 진단을
            //   내고도 종료 훅이 도는 동안 몇 초 더 산다(실측 +2.2s 진단 / +6.4s 종료). 그 시체를
            //   기다리면 창을 넘겨 실패가 성공으로 판정되므로 여기서 확정한다. 처분은 그대로 관측뿐이다
            //   — 자식은 자기 pump 가 EOF→finish 하고 reaper 가 거둔다(ADR-0082 "아무것도 죽지마").
            // ★그래서 몇 초간 「도는 중 + 마지막 실패」 조합이 화면에 선다 — 버그가 아니다★: 두 축이
            //   별개라 표현되는 상태이고, 트리는 「도는 중이 이긴다」로 그 사이를 그린다(ADR-0173).
            // ADR-0172
            EarlyVerdict::Diagnosed(kind) => {
                let reason = format!("resume 실패 진단({kind:?})");
                self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    ?kind,
                    "ADR-0172: 진단 스트림이 이어받기 실패를 확정 — 종료를 기다리지 않는다"
                );
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★프로세스는 살아 있는데 **쓸 수 없다** — 살아 있음을 성공으로 세지 않는 자리★.
            //
            // 이 갈래가 없던 시절의 결말이 ADR-0082 가 막으려던 바로 그것이었다: 이어받기가 거절당해도
            //   자식은 멀쩡하고 거절은 stdout 으로 오므로 창은 `Running` 만 보았고, 아래 `Alive` 로
            //   떨어져 **성공으로 보고되며 마지막 실패 기록까지 지웠다.**
            // ★증거가 `reason` 하나뿐이다★ — 시체가 없어 콘솔 꼬리도 진단 꼬리도 비어 있다. 그래서
            //   분류 입력으로 그 문자열을 그대로 넘긴다(무슨 뜻인지는 backend 지식 — ADR-0004).
            // ★★처분은 `Diagnosed` 와 **다르다 — 이 갈래는 거둔다**★★: 통로는 자기 쪽 stdin 을 놓는
            //   데까지만 하고(ADR-0199 의 선) 상대가 stdout 을 붙든 채 남으면 그 사슬이 안 돈다. 그래서
            //   아래에서 `tear_down_failed_activation` 이 기존 2 동사로 거둔다(그 함수의 doc 이 「왜
            //   매니저가 하나」의 정본).
            //   ★한때 여기 「아무것도 죽이지 않는다 · 매니저는 관측만 한다」로 적혀 있었는데, 열네 줄
            //   아래에 그 kill 이 있다★ — 그 문장은 통로가 유계로 기다렸다 스스로 거두던 라운드의 잔재다.
            //   ADR-0082 의 「아무것도 죽지마」와 어긋나지 않는 이유(관측된 시체 vs 종점이 한 번도 안 선
            //   화신)도 그 함수 doc 에 있다.
            EarlyVerdict::LinkFailed { reason } => {
                let kind = backend::resume_failure_kind(&profile.command, &reason)
                    .unwrap_or(AgentFailureKind::EarlyExitAfterResume);
                let reason = format!("resume 연결 실패: {reason}");
                // ★★정리를 **먼저** 한다 — 기록보다 앞이다★★: 기록은 프로필 뮤텍스를 잡는데, 그 뮤텍스는
                //   세션 id 기록 경로가 디스크 쓰기를 **쥔 채로** 들고 있을 수 있다. 순서를 뒤집으면
                //   그 정체가 곧 「자식을 영영 안 죽인다」가 된다 — 백스톱은 바로 그 정체를 위해 있는데
                //   그 정체 때문에 못 쓰게 되는 모양이다.
                //   ★한때 여기 「기록이 먼저여야 한다 — 먼저 죽이면 `Killed` 가 서서 사용자 종료와
                //   구별되지 않는다」로 적혀 있었는데 **그 사유는 틀렸다**★: 그 규율은
                //   `early_activation_verdict` 의 종점 갈래 안에 있고, 그 함수는 이미 판정을 내고
                //   돌아온 뒤다. 판정 **뒤에** 서는 상태는 그 판정을 바꾸지 못한다.
                self.tear_down_failed_activation(profile.id, spawned.epoch);
                self.note_activation_result(profile.id, Some(spawned.epoch), Some(kind));
                tracing::warn!(
                    agent = %profile.id,
                    %reason,
                    ?kind,
                    "ADR-0082: 통로가 연결을 못 세웠다 → 활성화 실패, fresh-fallback 없음 — LLM 에스컬레이션 대상"
                );
                (RestoreOutcome::Failed { reason }, Some(spawned))
            }
            // ★상대가 **준비됐다고 신호했다** = 성립(사용자 결정)★ — 창을 지켜본 것이 아니라 긍정 증거를
            //   받은 것이다. 그래서 이 갈래는 `Alive` 보다 **빨리**, 그리고 **강한 근거로** 도착한다.
            //
            // ★★그런데 「마지막 실패」는 **지우지 않는다**★★(사용자 조건): 이 성공은 낙관적이다 —
            //   에이전트가 입력을 받을 수 있다는 것까지가 관측이고, 그 뒤에 일어나는 일(턴 오류·연결
            //   끊김)은 **다른 경로가 지는 다른 사건**이다. 원래 결함의 절반이 「성공 보고가 실패 증거를
            //   함께 지운 것」이었고, 판정이 빨라진 만큼 그 파괴는 더 이르게 일어난다.
            // ★그래서 지움 규칙을 이렇게 둔다 — **자동으로는 아무것도 안 지운다**★:
            //   이 기록이 답하는 질문은 「직전 활성화가 어떻게 끝났나」이고, 그것을 무효로 만드는 증거는
            //   **다음 실패가 덮어쓰는 것**(`set_last_failure(Some(..))`)과 **프로필 자체가 사라지는 것**
            //   뿐이다. 「지금 떠 있다」는 그 질문에 대한 답이 아니다.
            //   ★대가를 정직하게 적는다★: 그래서 낡은 실패가 산 에이전트 옆에 남을 수 있다. 화면에서는
            //   그 조합이 「도는 중이 이긴다」로 그려지므로(ADR-0173) 오해로 이어지지 않고, 반대 방향의
            //   대가(증거 소실)는 되돌릴 수 없다 — 그래서 남기는 쪽을 고른다.
            // ★claude 는 이 갈래로 오지 않는다★ — 그쪽 통로는 연결 축이 없어 아래 `Alive` 로 간다.
            //   지움 규칙을 그쪽으로 넓히지 말 것(별건이고, 그 경로는 이 결정의 범위 밖이다).
            EarlyVerdict::Ready => {
                // ★여기서 두 결말이 갈린다★ — 활성화는 똑같이 성립했지만 **무엇이 성립했는지**가 다르다.
                //   `Started` 는 이미 있던 어휘다(「이어받기 대상이 아니라 새 세션을 시작함」) — 새 칸을
                //   만들지 않고 그 뜻 그대로 쓴다.
                let outcome = if opens_a_new_conversation {
                    RestoreOutcome::Started
                } else {
                    RestoreOutcome::Resumed
                };
                (outcome, Some(spawned))
            }
            EarlyVerdict::Alive => {
                // ★조기종료 창을 넘겼고 진단도 침묵했다 = 이어받을 대화가 실재했다★ — 여기가 「지움」의
                //   근거다(`note_activation_result` 주석).
                // ★이 갈래로 오는 것은 **연결 축이 없는 통로**뿐이다★(claude·shell·stdio) — 연결을
                //   선언하는 통로는 위 `Ready`/`LinkFailed` 에서 이미 갈린다. 그래서 지움 규칙이 여기
                //   남아 있는 것이 claude 경로를 그대로 두는 것과 같은 말이다.
                self.note_activation_result(profile.id, Some(spawned.epoch), None);
                let outcome = if opens_a_new_conversation {
                    RestoreOutcome::Started
                } else {
                    RestoreOutcome::Resumed
                };
                (outcome, Some(spawned))
            }
        }
    }

    /// 활성화가 **실패로 확정된** 세션을 끝낸다 — 프로세스가 아직 살아 있을 때만 의미가 있다.
    ///
    /// ★왜 매니저가 하나★: 이어받기가 거절되면 통로는 자기 쪽 stdin 만 놓고, 그 뒤 상대가 스스로 끝나기를
    ///   기다린다. 상대가 stdout 을 붙든 채 남으면 리더가 EOF 를 못 봐 `OutputCore::finish` 가 영영 안 돌고,
    ///   그 메시지의 단독 소비자인 reaper 도 움직이지 않는다 — 자식·Job·스레드가 데몬 수명 내내 묶인다.
    ///   그 처분의 주인은 **kill 핸들러**이고(ADR-0001 의 2 동사), 통로가 아니다.
    ///   ★한때 통로의 라이터가 직접 kill 하던 자리가 있었는데 그것이 ADR-0199 가 그은 선을 넘는 것이라
    ///   걷어냈다 — 되살리지 말 것★. 여기가 그 대체이고, 새로 발명한 동사가 아니라 이미 있는 teardown 이다.
    /// ★`kill_agent` 를 그대로 부르지 않는 이유 = **사용자 종료 표식**★: 그쪽은
    ///   `TerminationIntent::UserKill` 을 세우는데 이건 사용자가 끈 것이 아니다. 그 표식은 `ReapMsg` 에
    ///   frozen 으로 실려 나가므로, 거짓으로 세우면 종료 분류 기록이 통째로 거짓이 된다.
    /// ★ADR-0082 「아무것도 죽지마」와 충돌하지 않는다★ — 그 결정이 지키는 것은 **관측된 종점(시체)** 이고,
    ///   여기서 끝내는 것은 종점이 **한 번도 서지 않은** 화신이다. 죽이는 것이 증거를 지우는 게 아니라
    ///   **없던 증거(종료 전이)를 만드는** 쪽이다. 실패 사유는 이 호출 **전에** 이미 기록된다.
    /// ★그래서 `Terminal`·`Diagnosed` 갈래에서는 부르지 않는다★ — 그쪽은 자식이 이미 죽었거나(시체 보존),
    ///   claude 가 종료 훅을 도는 중이라 ADR-0082 가 그대로 적용된다.
    fn tear_down_failed_activation(&self, id: AgentId, incarnation: u32) {
        let Ok(session) = self.get_session(id) else {
            return;
        };
        // ★★표식이 다르면 **손대지 않는다**★★: 이 정리는 판정만큼 늦게 도착하고, 그 사이 이 화신이
        //   수거되고 **다음 화신이 떴을 수 있다.** id 로만 찾아 죽이면 그때 죽는 것은 실패한 세션이 아니라
        //   **막 뜬 건강한 후임**이고, `revoke` 도 그 후임의 토큰을 지운다.
        //   ★비교는 일치/불일치뿐★ — 대소로 「더 새 것」을 유도하지 않는다(ADR-0163/0164).
        if session.epoch != incarnation {
            tracing::info!(
                agent = %id,
                expected = incarnation,
                live = session.epoch,
                "실패한 활성화의 정리를 건너뛴다 — 그 사이 다른 화신이 섰다(산 세션을 죽이지 않는다)"
            );
            return;
        }
        self.control.revoke(id, session.epoch);
        let _ = session.enter_exiting();
        session.kill(Duration::from_secs(5));
    }

    /// spawn 후 window 안에 이어받기의 결말을 판정한다 — **죽음 또는 증거**(`EarlyVerdict`).
    ///
    /// ★증거를 죽음보다 먼저 보지 않는다(루프 안 순서 — load-bearing)★: 매 폴에서 상태를 먼저 보고
    ///   그 다음 진단을 본다. 뒤집으면 창 안에 온 **사용자 종료**가 `Diagnosed` 로 앞질러 잡혀,
    ///   "사용자가 끊은 것은 실패가 아니다" 규율(아래 `resume_no_fallback`)이 우회된다.
    /// ★진단만으로 확정하는 것은 stderr 스트림뿐이다 — 콘솔 링을 여기에 넣지 마라★: 콘솔 링에는
    ///   claude 가 이어받아 **다시 그린 대화 본문**이 통째로 들어온다. 살아 있는 세션의 링에서 실패
    ///   문구를 찾으면, 예전에 그 문구를 이야기한 대화(이 저장소의 세션이 실제로 그렇다)를 이어받는
    ///   순간 **성공한 활성화가 실패로 도장 찍힌다.** stderr 는 claude 자신의 진단 채널이라 대화 본문이
    ///   섞이지 않으므로 그 오탐이 원천적으로 없다. 콘솔 꼬리는 **죽은 뒤**에만 증거로 쓴다(아래).
    /// ★세션 Arc 를 **루프 밖에서 한 번** 잡는다(load-bearing — 되돌리지 마라)★: 루프 안에서 매번
    ///   `get_session` 하면 거의 항상 꼬리를 놓친다. `finish` 는 terminal 상태를 세운 **직후** reaper 를
    ///   깨우고 reaper 는 첫 동작으로 세션을 명부에서 지우는데, 이 루프의 간격은 100ms 다 — 즉 우리가
    ///   깨어날 때쯤엔 이미 조회가 실패해 빈 꼬리로 떨어지고, 분류는 언제나 맥락 기본값이 된다
    ///   (그러면 「이어받을 대화 없음」 판정이 사실상 죽은 코드가 된다). Arc 를 들고 있으면 명부에서
    ///   빠진 뒤에도 같은 `OutputCore` 를 보므로 terminal 상태도 출력 링도 진단 버퍼도 그대로 읽힌다.
    /// ★첫 조회 실패는 "세션이 없었다" 가 아니다★: `spawn_agent` 가 Ok 를 준 뒤 이 조회에 닿기까지 자식이
    ///   죽어 reaper 가 이미 거둬 갔을 수 있다(빠른 즉사 배치가 실제로 그 순서를 낸다). 그 경우 꼬리를
    ///   놓치므로 분류는 맥락 기본값으로 떨어진다 — 어느 쪽이든 재시도 가능 쪽이라 fail-open 이고, 그래서
    ///   여기서 두 원인을 가르지 않는다.
    /// ★증거 두 갈래는 배타적이고, 그래서 합쳐서 넘긴다★: PTY(터미널) 세션은 stderr 가 콘솔에 병합돼
    ///   링에만 있고, 파이프(구조화) 세션은 진단 버퍼에만 있다. 어느 쪽이든 빈 값이 정상 결과이며,
    ///   둘 다 비면 호출자가 맥락 기본값으로 떨어뜨린다(fail-open).
    /// ★`command` 를 받는 이유★: "이 문구가 무슨 뜻인가" 는 백엔드 지식이라 판정을 `backend` 에 위임한다
    ///   (ADR-0004) — manager 는 문자열을 직접 읽지 않는다.
    // ADR-0172
    fn early_activation_verdict(
        &self,
        id: AgentId,
        command: &AgentCommand,
        window: Duration,
        // ★이 갈래는 연결 축이 없는 통로 전용이라 백스톱을 쓰지 않는다★ — 인자를 남겨 두는 것은 두
        //   판정의 호출 모양을 같게 유지하기 위함이고, 쓰이지 않는다는 사실이 그 자체로 계약이다.
        _link_backstop: Duration,
    ) -> EarlyVerdict {
        let session = match self.get_session(id) {
            Ok(s) => s,
            // 명부에 없다 = 아직 안 올랐거나 이미 거둬졌다 → 어느 쪽이든 종료로 간주(위 doc).
            // ★이 `AgentStatus::Failed` 는 **공표되지 않는다**★: `StatusSink` 에 닿지 않는 지역값이라
            //   화면·프론트에는 안 나가고, 아래 `format!("resume 조기 종료({status:?})")` 의 사유 문자열과
            //   `matches!(status, AgentStatus::Killed)` 판정에만 쓰인다. 그래서 "공표되는 `Failed` 는
            //   pump 패닉 전용"이 그대로 성립한다 — 다만 *코드에 두 곳뿐*이라고 적으면 여기서 거짓이 된다
            //   (문서가 그렇게 적었다가 2026-08-25 에 정정됐다. 정본 = architecture-overview 「세션 복원 / 활성화」).
            Err(_) => {
                return EarlyVerdict::Terminal {
                    status: AgentStatus::Failed {
                        message: "session gone".into(),
                    },
                    evidence: String::new(),
                }
            }
        };
        // ★연결 축이 **없는** 통로만 여기로 온다★ — 축이 있는 쪽은 아래
        //   [`AgentManager::link_activation_verdict`] 가 배달을 기다린다. 이 갈래는 claude·shell·stdio·
        //   codex 터미널의 길이고, 라운드 2 이전과 **바이트 단위로 같다**(창 하나, 결말 `Alive`).
        let deadline = Instant::now() + window;
        loop {
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return EarlyVerdict::Terminal {
                    status,
                    evidence: Self::terminal_evidence(&session),
                };
            }
            if let Some(kind) = backend::resume_failure_kind(command, &session.diagnostic_tail()) {
                return EarlyVerdict::Diagnosed(kind);
            }
            if Instant::now() >= deadline {
                return EarlyVerdict::Alive;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 사용자 취소를 판정으로 옮긴다 — ★`Killed` 를 **여기서 세운다**★.
    ///
    /// ★이 `AgentStatus::Killed` 는 공표되지 않는다★: `StatusSink` 에 닿지 않는 지역값이라 화면·프론트로
    ///   나가지 않고, 호출자의 사유 문자열과 `matches!(status, AgentStatus::Killed)` 판정에만 쓰인다
    ///   (`early_activation_verdict` 의 `"session gone"` 값과 같은 부류·같은 근거).
    /// ★「이미 죽었다」가 아니라 「사용자가 끊기로 했다」를 뜻한다★ — 그 순간 자식은 아직 살아 있을 수
    ///   있다(`session.kill()` 이 도는 중이다). 우리가 단언하는 것은 **이 활성화의 결말을 사용자가
    ///   정했다**는 것 하나이고, 호출자가 그 값으로 하는 일은 「실패로 기록하지 않는다」뿐이다.
    /// `delivered` = 이 취소가 가로챈 배달의 사유(있으면). 증거 꼬리에 이어 붙인다 — 그 문구는 콘솔에도
    /// 진단에도 없어서, 여기서 안 실으면 어디에도 안 남는다.
    fn cancelled_verdict(session: &AgentSession, delivered: Option<String>) -> EarlyVerdict {
        let mut evidence = Self::terminal_evidence(session);
        if let Some(reason) = delivered {
            if !evidence.is_empty() {
                evidence.push('\n');
            }
            evidence.push_str(&reason);
        }
        EarlyVerdict::Terminal {
            status: AgentStatus::Killed,
            evidence,
        }
    }

    /// 배달 한 건에서 **실패 사유 문자열만** 꺼낸다 — 표식이 다르거나 성공이면 `None`.
    fn delivered_reason(v: LinkVerdict, epoch: u32) -> Option<String> {
        if v.epoch != epoch {
            return None;
        }
        match v.resolution {
            LinkResolution::Failed { reason } => Some(reason),
            LinkResolution::Ready => None,
        }
    }

    /// 종점 증거 — 콘솔 꼬리 + 진단(stderr) 꼬리. 두 스트림은 통로에 따라 배타적으로 찬다.
    fn terminal_evidence(session: &AgentSession) -> String {
        let tail = session.terminal_tail(FAILURE_TAIL_BYTES);
        let mut evidence = String::from_utf8_lossy(&tail).into_owned();
        let diagnostics = session.diagnostic_tail();
        if !diagnostics.is_empty() {
            evidence.push('\n');
            evidence.push_str(&diagnostics);
        }
        evidence
    }

    /// 연결을 세우는 통로의 활성화 판정 — ★상태를 **읽지** 않고 결말이 **배달되기를** 기다린다★.
    ///
    /// ★이 함수가 따로 있는 이유★: 읽기 기반 판정과 배달 기반 판정은 같은 루프에 못 들어간다. 섞으면
    ///   「읽은 뒤 행동하기까지의 틈」이 되살아나고, 그 틈이 곧 이 라운드가 닫은 결함 넷이다. 그리고
    ///   축이 없는 backend 의 길을 **한 줄도** 건드리지 않는 것이 이 분리의 두 번째 값어치다.
    ///
    /// 네 가지만 본다 — ★첫째가 **나머지 셋보다 먼저**다★:
    ///   1. **사용자 취소 래치** — `kill_agent` 가 통로를 내리기 **전에** 세운다. 배달·채널 소멸·상태
    ///      어느 것보다 먼저 보이므로, 이것을 먼저 보지 않으면 사용자의 취소가 실패나 성공으로 둔갑한다
    ///      (아래 두 갈래의 주석이 그 두 둔갑의 정본).
    ///   2. **배달된 결말** — 한 번 보내지면 철회되지 않는다. 화신 표식이 이 spawn 의 것일 때만 받는다.
    ///   3. **종점 상태** — 래치다(`OutputCore` 가 한 번만 세운다). 그래서 읽어도 뒤집히지 않는다.
    ///   4. **백스톱** — 통로가 결말을 **못 내는** 경로의 liveness 장치이지 판정 창이 아니다
    ///      ([`LINK_RESOLUTION_BACKSTOP`]).
    ///
    /// ★★종점이 배달을 이기지는 **않는다** — 취소만 이긴다★★: `Ready` 가 실제로 배달됐다면 상대는
    ///   준비됐다고 **신호한 것**이고, 그 뒤에 스스로 죽는 것은 ADR-0201 이 「별개 사건」으로 못박은
    ///   그것이다(pump → 종점 → reaper 가 본다). 그래서 자기 죽음으로 성공 판정을 뒤집지 않는다 —
    ///   대신 호출자가 **그 순간의 상태를 다시 떠서** 돌려준다(`spawn_fresh_settled` 의 `Ready` 갈래 ·
    ///   `activate_profile` 의 Resume 갈래). 사용자 취소는 다르다: 그건 이 활성화의 결말을 **사람이
    ///   정한 것**이라 판정 자체가 바뀐다.
    /// ★`epoch` 을 받아 **일치할 때만** 받는다★ — 오늘 그 불일치가 실제로 생기지 않는 이유와 그래도
    ///   남겨 두는 이유는 아래 `as_verdict` 의 doc 이 갖는다(여기 되풀어 적지 않는다).
    /// ★백스톱 직전에 **한 번 더 배달을 확인한다**★ — 시한과 배달이 겹치면 배달이 이긴다. 안 그러면
    ///   막 성립한 연결이 실패로 기록되고 죽는다(결함 ②: 원래 결함의 부호만 뒤집힌 판).
    fn link_activation_verdict(
        &self,
        id: AgentId,
        epoch: u32,
        link_rx: &Receiver<LinkVerdict>,
        backstop: Duration,
    ) -> EarlyVerdict {
        /// 배달 한 건을 판정으로 옮긴다 — 표식이 다르면 **버린다**(다른 화신의 결말이다).
        ///
        /// ★★이 가드는 **오늘 운영에서 발화하지 않는다 — 그것을 알고 남긴다**★★: 배달함은 spawn 마다
        ///   새로 만들고([`AgentManager::spawn_agent_watching_link`]) 표식은 그때 포트 클로저에 구워
        ///   넣으므로, 이 `rx` 로 오는 모든 메시지는 정의상 우리가 기다리는 그 표식을 단다. 즉 오늘
        ///   불일치를 막는 실물은 이 `if` 가 아니라 **채널의 신선도**다.
        /// ★그래도 지우지 않는 이유★: 채널을 공유하게 만드는 변경(한 감독자가 여러 spawn 을 보거나,
        ///   포트를 재사용하거나)이 오면 그 순간 이것이 유일한 벽이 된다. 값싸고 옳다.
        /// ★한때 이 자리에 「수거된 화신의 결말이 막 뜬 후임에게 적용되는 것을 이 가드가 막는다」로
        ///   적혀 있었는데 **그렇게 읽으면 거짓이다**★ — 그 일이 안 일어나는 진짜 이유가 채널 신선도라,
        ///   그 문장을 믿는 사람은 한 번도 발화한 적 없는 가드에 기대게 된다. 발화 경로는 시험대에만
        ///   있다(`a_verdict_from_another_incarnation_is_discarded` 가 채널을 손으로 만든다).
        fn as_verdict(v: LinkVerdict, epoch: u32) -> Option<EarlyVerdict> {
            if v.epoch != epoch {
                tracing::debug!(
                    delivered = v.epoch,
                    expected = epoch,
                    "다른 화신의 연결 결말이 도착했다 — 버린다"
                );
                return None;
            }
            Some(match v.resolution {
                LinkResolution::Ready => EarlyVerdict::Ready,
                LinkResolution::Failed { reason } => EarlyVerdict::LinkFailed { reason },
            })
        }

        /// ★★사용자가 이 화신을 끊었나 — **배달보다 먼저 보는 래치**★★.
        ///
        /// ★종점 상태로는 이것을 못 본다★: `kill_agent` 는 `set_intent(UserKill)` → `enter_exiting()`
        ///   → `session.kill()` 순서로 가고, 종점 전이(`Killed`)는 그 뒤 pump 가 깨어야 선다. 그
        ///   사이의 상태는 `Exiting` 이라 **종점이 아니고**, 그 구간에 통로가 사라지면 아래 `Disconnected`
        ///   갈래가 그것을 「연결 실패」로 적는다 — 사용자의 취소가 이어받기 실패로 둔갑한다.
        /// ★이 래치는 그 모든 관측보다 **반드시 먼저** 보인다★ — 위 순서가 그것을 보장한다
        ///   (`AgentSession::termination_intent` 의 doc 이 그 인과의 정본).
        fn user_cancelled(session: &AgentSession) -> bool {
            matches!(session.termination_intent(), TerminationIntent::UserKill)
        }

        let session = match self.get_session(id) {
            Ok(s) => s,
            Err(_) => {
                return EarlyVerdict::Terminal {
                    status: AgentStatus::Failed {
                        message: "session gone".into(),
                    },
                    evidence: String::new(),
                }
            }
        };
        let deadline = Instant::now() + backstop;
        loop {
            // ★기다림을 슬라이스로 끊는 이유는 **종점 래치를 같이 보기 위해서**다★ — 배달은 이 대기를
            //   즉시 깨우므로 슬라이스 길이가 성공 지연을 만들지 않는다.
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            match link_rx.recv_timeout(slice) {
                Ok(v) => {
                    // ★★사용자 취소가 **배달을 이긴다**★★: 핸드셰이크가 성공해 `Ready` 가 큐에 선
                    //   직후 사용자가 끊으면, 이 자리에서 그 `Ready` 를 그대로 받아 **죽은 에이전트를
                    //   성공으로 보고**했다(`restore_one` 은 `Resumed`, `activate_profile` 은 `Ok`).
                    //   래치를 먼저 보면 그 조합이 「사용자가 끈 것」으로 떨어져 기록도 보고도 정직해진다.
                    // ★버린 배달의 사유는 증거로 옮겨 싣는다★ — 그 문구는 두 꼬리 어디에도 없다.
                    if user_cancelled(&session) {
                        return Self::cancelled_verdict(&session, Self::delivered_reason(v, epoch));
                    }
                    if let Some(verdict) = as_verdict(v, epoch) {
                        return verdict;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // ★★래치를 **상태보다 먼저** 본다 — 순서를 되돌리지 마라★★: 통로가 사라지는 흔한
                    //   원인 하나가 **사용자 kill 그 자체**다. kill 은 배달을 억제하고(통로의
                    //   `deliver_link`) 라이터를 끝내는데, 라이터가 끝나면 보내는 끝이 떨어져 이 갈래가
                    //   깨어난다 — 그때 `session.kill()` 은 아직 `child.kill()`/`wait()` 안이고 pump 는
                    //   `finish` 를 못 돌았으므로 상태는 `Exiting`(**종점이 아니다**)이다.
                    //   상태만 보던 옛 모양은 그 한 번의 읽기로 「통로가 결말을 배달하지 못한 채
                    //   사라졌다」를 확정했고, 그래서 **사용자의 취소가 이어받기 실패로 기록됐다** —
                    //   억제 장치가 막으려던 바로 그 결말이다.
                    // ★유예를 두지 않는 것이 의도다★ — 이 래치는 kill 경로에서 통로 소멸보다 **반드시
                    //   먼저** 서므로 기다릴 이유가 없고, kill 이 아닌 소멸에서는 유예를 줘도 결말이
                    //   실패로 같다(문구만 갈린다).
                    if user_cancelled(&session) {
                        return Self::cancelled_verdict(&session, None);
                    }
                    // 통로가 배달 없이 사라졌다 — 남은 사실은 종점 상태뿐이다.
                    let status = session.status();
                    if matches!(
                        status,
                        AgentStatus::Exited { .. }
                            | AgentStatus::Killed
                            | AgentStatus::Failed { .. }
                    ) {
                        return EarlyVerdict::Terminal {
                            status,
                            evidence: Self::terminal_evidence(&session),
                        };
                    }
                    return EarlyVerdict::LinkFailed {
                        reason: "통로가 연결의 결말을 배달하지 못한 채 사라졌다".to_string(),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
            }

            // ★사용자 kill 이 여기로도 온다★ — 위 두 갈래와 같은 규율이고, 종점 전이를 기다리지 않는다.
            if user_cancelled(&session) {
                return Self::cancelled_verdict(&session, None);
            }

            // ★종점은 래치라 읽어도 안전하다★ — `OutputCore` 가 한 번만 세우고 되돌리지 않는다.
            let status = session.status();
            if matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                let mut evidence = Self::terminal_evidence(&session);
                // 종점이 배달을 앞질렀어도 사유는 살린다 — 그 문구는 두 꼬리 어디에도 없다.
                if let Ok(v) = link_rx.try_recv() {
                    if v.epoch == epoch {
                        if let LinkResolution::Failed { reason } = v.resolution {
                            if !evidence.is_empty() {
                                evidence.push('\n');
                            }
                            evidence.push_str(&reason);
                        }
                    }
                }
                return EarlyVerdict::Terminal { status, evidence };
            }

            if Instant::now() >= deadline {
                // ★포기하기 전에 **마지막으로** 배달함을 본다★ — 결함 ② 가 여기서 닫힌다.
                if let Ok(v) = link_rx.try_recv() {
                    if let Some(verdict) = as_verdict(v, epoch) {
                        return verdict;
                    }
                }
                return EarlyVerdict::LinkFailed {
                    reason: format!(
                        "통로가 {}초 안에 연결의 결말을 내지 못했다 — 상대가 답도 하지 않고 우리 쓰기도                          받지 않는다(핸드셰이크 상한은 응답 대기에만 걸린다)",
                        backstop.as_secs()
                    ),
                };
            }
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
    ) -> Result<crate::types::WriteOutcome, PtyError> {
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
    ) -> Result<crate::types::WriteOutcome, PtyError> {
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
    ) -> Result<crate::types::WriteOutcome, PtyError> {
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
        classify: crate::backend::TurnClassifier,
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
        crate::name::canonical_name_or_id_fallback(display_name.as_deref(), &cwd, session.id)
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
        let name = crate::name::canonical_name_or_id_fallback(display_name, &cwd, session.id);
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

    // ── ADR-0185: 「받아 온 세션 id」 기록 동사 ────────────────────────────────────────────
    //
    // 여기서 재는 것은 `session_id_sink` 가 만든 동사 하나다 — 그 동사가 실제로 spawn 에 실리는지는
    //   `backend/codex/mod.rs` 의 app-server 종단 항목이 잰다.

    /// ★저장소가 **메모리**인 것은 의도다★ — 이 세 항목이 단언하는 것은 레지스트리의 판정뿐이라
    /// 영속이 무관하고, `bare_manager` 처럼 파일 저장을 쓰면 호출마다 temp 폴더가 하나씩 남는다
    /// (한 번의 crate 회귀에 9개가 쌓였다 — 실측).
    fn sink_fixture() -> (Arc<crate::profile::ProfileRegistry>, AgentId, u32) {
        #[derive(Default)]
        struct MemStore(Mutex<Vec<AgentProfile>>);
        impl crate::profile::ProfileStore for MemStore {
            fn save(&self, profiles: &[AgentProfile]) {
                *self.0.lock().expect("mem store poisoned") = profiles.to_vec();
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.lock().expect("mem store poisoned").clone()
            }
        }
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            MemStore::default(),
        )));
        let p = AgentProfile::new(
            "raw".into(),
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::StreamJson,
            },
            std::path::PathBuf::from("."),
            vec![],
            true,
        );
        let id = p.id;
        profiles.upsert(p);
        let epoch = profiles.epoch_for_spawn(id).expect("갓 넣은 프로필");
        (profiles, id, epoch)
    }

    #[test]
    fn the_session_id_sink_records_the_id_it_is_handed() {
        let (profiles, id, epoch) = sink_fixture();
        let sid = uuid::Uuid::new_v4();

        session_id_sink(profiles.clone(), id, epoch)(&sid.to_string());

        assert_eq!(profiles.get(id).unwrap().backend_session_id, Some(sid));
    }

    /// ★죽은 화신의 뒤늦은 기록이 산 화신의 값을 덮지 못한다★ — 이 동사는 자기 spawn 의 표식을 들고
    /// 있고, 그 표식은 다음 spawn 이 새로 뽑으면서 낡는다.
    #[test]
    fn a_dead_incarnations_sink_cannot_overwrite_a_live_value() {
        let (profiles, id, dead_epoch) = sink_fixture();
        let dead_sink = session_id_sink(profiles.clone(), id, dead_epoch);

        // 새 화신이 서고 자기 값을 적는다.
        let live_epoch = profiles.epoch_for_spawn(id).expect("프로필은 그대로다");
        let live_sid = uuid::Uuid::new_v4();
        session_id_sink(profiles.clone(), id, live_epoch)(&live_sid.to_string());

        // 죽은 화신의 통로가 이제야 자기 thread id 를 들고 돌아온다.
        dead_sink(&uuid::Uuid::new_v4().to_string());

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            Some(live_sid),
            "죽은 화신의 기록이 산 화신의 값을 덮었다"
        );
    }

    /// uuid 로 못 읽히는 값은 **버린다** — panic 하면 그 라이터 스레드가 죽어 세션이 통째로 말을 잃는다.
    #[test]
    fn a_non_uuid_session_id_is_dropped_without_panicking() {
        let (profiles, id, epoch) = sink_fixture();
        let sink = session_id_sink(profiles.clone(), id, epoch);

        sink("not-a-uuid");
        sink("");

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            None,
            "해독 실패한 값이 프로필에 적혔다"
        );
    }

    /// ★버리는 것의 **대가**를 잰다 — 「안 적혔다」가 아니라 「갈렸다」다★.
    ///
    /// 위 항목은 빈 프로필에서 시작해서 「버렸다」와 「원래 없었다」가 구별되지 않는다. 실제로 아픈 모양은
    /// 이쪽이다: 저장된 손잡이가 **이미 있는데** 상대가 uuid 아닌 thread id 를 주면, 통로는 그 문자열을
    /// 들고 그 뒤 모든 턴을 그것으로 내보내는데 디스크는 옛 값 그대로 남는다. 두 축이 조용히 갈리고
    /// 다음 이어받기는 이 화신이 실제로 말한 스레드가 아닌 곳을 연다 — 화면에는 아무 표시도 없다.
    ///
    /// ★그 조건을 만드는 짝은 `backend/codex/mod.rs` 의
    ///   `tests::a_resume_target_makes_the_handshake_issue_thread_resume` 이다★ — 그 시험대의 가짜가
    ///   `resumed-<uuid>` 를 돌려주는데 그 문자열은 uuid 가 아니다(그쪽이 그 사실을 단언한다).
    /// ★이 항목은 「버려야 한다」를 뒤집자는 것이 아니다★ — 여기서 panic 하거나 옛 값을 지우면 더 나쁘다
    ///   (그 사유는 [`session_id_sink`] 의 doc). 못 박는 것은 **이 결말에 값이 있다**는 사실이고,
    ///   그래서 이 갈래가 조용해질 때 회귀가 아니라 **결정**이 되도록 남긴다.
    #[test]
    fn a_non_uuid_thread_id_leaves_the_stored_one_diverged() {
        let (profiles, id, epoch) = sink_fixture();
        let stored = profiles.ensure_session_id(id).expect("갓 넣은 프로필");

        session_id_sink(profiles.clone(), id, epoch)("resumed-abc");

        assert_eq!(
            profiles.get(id).unwrap().backend_session_id,
            Some(stored),
            "해독 실패한 값이 저장된 손잡이를 건드렸다 — 버리기로 한 결정이 깨졌다"
        );
    }

    // ── ADR-0044/0191: backend 가 넘겨준 통로가 무엇을 나르나 ──────────────────────────────
    //
    // ★통로 **타입**이 아니라 통로가 신고하는 caps 로 잰다★: 만드는 코드가 `backend/<이름>/` 안으로
    //   들어가 조립점에는 이름으로 부를 타입이 없다(ADR-0191). 재는 사실은 그대로다.
    // ★`probe_spec` 은 실 CLI 가 아니다★: 즉시 끝나는 프로브를 그 backend 의 통로 선택으로 띄워, 어느
    //   구현체가 골라졌는지만 본다(ADR-0012 격리 — 실 claude 바이너리 불요).
    #[cfg(windows)]
    #[test]
    fn stream_json_backend_hands_over_structured_pipe() {
        let parts = backend::open_spawn(
            &claude_stream_json_command(),
            &probe_spec(),
            DEFAULT_COLS,
            DEFAULT_ROWS,
            None,
            None,
            None,
        )
        .expect("open_spawn");
        let caps = parts.transport.capabilities();
        assert!(
            caps.output.structured && !caps.output.terminal_bytes,
            "stream-json → 구조화 파이프(structured 출력, 터미널 바이트 아님)"
        );
        assert!(!caps.control.resize, "파이프 resize 불가");
        parts.transport.shutdown();
    }

    // ── 회귀 ──
    #[cfg(windows)]
    #[test]
    fn terminal_backend_hands_over_pty() {
        let parts = backend::open_spawn(
            &claude_terminal_command(),
            &probe_spec(),
            DEFAULT_COLS,
            DEFAULT_ROWS,
            None,
            None,
            None,
        )
        .expect("open_spawn");
        let caps = parts.transport.capabilities();
        assert!(
            caps.output.terminal_bytes && !caps.output.structured,
            "터미널 모드 → PTY(터미널 바이트, 구조화 아님)"
        );
        assert!(caps.control.resize, "PTY resize 가능");
        parts.transport.shutdown();
    }

    // ── write_stdin_observed_if_epoch ──
    //
    // ★왜 실 spawn 없이 세션을 맵에 직접 꽂나★: 검증 대상은 "맵이 가리키는 세션의 epoch 과 요구 epoch 을
    //   비교해 write 를 집행/거부하는가" 뿐이라, 실 자식·PTY·claude 바이너리가 전부 무관하다(ADR-0012 격리).
    //   in-crate 테스트라 private `sessions` 에 직접 접근한다 — `insert_test_session`(feature gate) 불요.

    use crate::backend::InputEncoder;
    use crate::persistence::{FilePresetStore, FileProfileStore};
    use crate::transport::AgentTransport;
    use crate::types::{
        BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps, OutputCaps, SessionCaps,
        TransportCaps,
    };

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

    /// 연결을 **세워야 하는** 통로의 대역 — `link_state` 를 밖에서 갈아 끼울 수 있다.
    ///
    /// ★실 codex 없이 재는 이유★: 재려는 것은 「판정이 그 축을 보나」이고, 그 답에 실 CLI 는 무관하다
    ///   (ADR-0012 격리). 실물로 재려면 codex 바이너리 + 거절당할 스레드 id 가 필요해 CI 에서 못 돈다.
    struct LinkedTransport {
        /// `shutdown()` 이 몇 번 불렸나 — 실패한 활성화의 teardown 이 실제로 통로까지 가는지 잰다.
        shutdowns: Arc<Mutex<usize>>,
    }
    impl AgentTransport for LinkedTransport {
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
        fn shutdown(&self) {
            *self.shutdowns.lock().expect("shutdowns poisoned") += 1;
        }
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: false,
                    message: true,
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
                    interrupt: true,
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
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            FileProfileStore::new(std::env::temp_dir().join(format!("engram-epoch-w-{tag}"))),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-epoch-w-preset-{tag}")),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::session_tracker::TrackerConfig {
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

    /// 명부에 **살아 있는**(종점 상태가 아닌) 세션 하나를 꽂고 그 화신의 `OutputCore` 를 돌려준다.
    ///
    /// ★실 프로세스를 쓰지 않는 이유★: 여기서 재는 것은 "진단 텍스트가 있고 세션이 살아 있을 때 판정이
    ///   어떻게 서는가" 뿐이라 자식·PTY·claude 바이너리가 전부 무관하다(ADR-0012 격리). `RecordingTransport`
    ///   는 아무것도 띄우지 않으므로 이 세션은 **스스로 절대 죽지 않는다** — 그게 "죽음을 기다리지 않는다"
    ///   를 재는 데 필요한 성질이다(실 claude 로는 그 상태를 결정적으로 붙들 수 없다).
    // ADR-0172
    fn put_live_session(manager: &AgentManager, id: AgentId, epoch: u32) -> Arc<OutputCore> {
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
                    resume: true,
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
            core.clone(),
            Box::new(RecordingTransport {
                written: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        core
    }

    /// 연결 축이 있는 통로를 단 산 세션을 명부에 꽂는다 — 돌려주는 것은 코어와 kill 계수기다.
    ///
    /// ★연결 상태를 들려 보내지 않는다★ — 이제 결말은 **배달**되고, 시험대는 그 채널을 직접 쥔다.
    fn put_session_with_link(
        manager: &AgentManager,
        id: AgentId,
        epoch: u32,
    ) -> (Arc<OutputCore>, Arc<Mutex<usize>>) {
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let shutdowns = Arc::new(Mutex::new(0usize));
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from("."),
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            BackendCaps {
                session: SessionCaps {
                    resume: true,
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
            core.clone(),
            Box::new(LinkedTransport {
                shutdowns: shutdowns.clone(),
            }),
        ));
        manager
            .sessions
            .write()
            .expect("sessions poisoned")
            .insert(id, session);
        (core, shutdowns)
    }

    fn link_epoch_fixture(manager: &AgentManager, epoch: u32) -> (AgentId, Arc<Mutex<usize>>) {
        let id = AgentId::new_v4();
        let (_core, kills) = put_session_with_link(manager, id, epoch);
        (id, kills)
    }

    /// ★배달된 `Ready` 는 **그 자리에서** 판정을 끝낸다★ — 어떤 창도 기다리지 않는다(사용자 결정).
    ///
    /// 백스톱을 크게 주고 경과를 재는 것이 요점이다: 판정이 다시 시간을 기다리기 시작하면 여기서 잡힌다.
    #[test]
    fn a_delivered_ready_ends_the_verdict_at_once() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 7);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 7,
            resolution: LinkResolution::Ready,
        })
        .expect("배달");

        let started = Instant::now();
        let verdict = manager.link_activation_verdict(id, 7, &rx, Duration::from_secs(6));
        let elapsed = started.elapsed();

        assert!(
            matches!(verdict, EarlyVerdict::Ready),
            "배달된 준비 신호가 성공으로 읽히지 않았다 — got {verdict:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "이미 배달된 결말을 두고 판정이 기다렸다({elapsed:?}) — 성공의 근거는 신호이지 경과 시간이 아니다"
        );
    }

    /// ★배달된 실패는 **사유를 들고** 온다★ — 그 문구가 분류의 유일한 입구다(두 꼬리에는 없다).
    #[test]
    fn a_delivered_failure_carries_the_reason_to_the_classifier() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 3);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 3,
            resolution: LinkResolution::Failed {
                reason: "thread/resume: -32600 no rollout found for thread id".into(),
            },
        })
        .expect("배달");

        let EarlyVerdict::LinkFailed { reason } =
            manager.link_activation_verdict(id, 3, &rx, Duration::from_secs(6))
        else {
            panic!("배달된 실패가 실패로 읽히지 않았다");
        };
        assert_eq!(
            backend::resume_failure_kind(&codex_app_server_command(), &reason),
            Some(AgentFailureKind::NoConversationToResume),
            "사유가 분류에 닿지 않는다 — 그러면 원인 대신 맥락 기본값이 마지막 실패에 찍힌다"
        );
    }

    /// ★★다른 화신의 결말은 **버린다**★★ — 이것이 「수거된 화신의 정리가 산 후임을 죽인다」의 뿌리다.
    ///
    /// 표식이 값에 붙어 있지 않던 시절에는 감독자가 id 로만 찾았고, 그 사이 화신이 바뀌면 옛 결말이
    /// 새 세션의 판정이 됐다. 여기서는 불일치가 조용히 버려지고, 판정은 **아무 일도 없었던 것처럼**
    /// 백스톱까지 간다.
    #[test]
    fn a_verdict_from_another_incarnation_is_discarded() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 11);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 10,
            resolution: LinkResolution::Ready,
        })
        .expect("배달");

        let verdict = manager.link_activation_verdict(id, 11, &rx, Duration::from_millis(200));
        assert!(
            matches!(verdict, EarlyVerdict::LinkFailed { .. }),
            "죽은 화신의 결말이 이 화신의 판정으로 쓰였다 — 그 뒤 정리가 산 세션을 죽인다: got {verdict:?}"
        );
    }

    /// ★배달된 값은 **시한에 지지 않는다**★ — 결함 ②(원래 결함의 부호만 뒤집힌 판)가 여기서 닫힌다.
    ///
    /// 백스톱이 이미 만료한 상태로 불러도, 채널에 값이 있으면 그 값이 판정이다. 옛 모양은 상태를 읽고
    /// 그 뒤 시한을 보았기 때문에 **막 성립한 연결이 실패로 기록되고 죽었다**.
    /// ★여기서 재는 것은 「값이 있으면 절대 시한으로 지지 않는다」는 성질이다★ — 시한과 배달이 정확히
    ///   겹치는 순간을 시험대가 만들 수는 없으므로, 그 성질을 만료된 시한으로 대신 확인한다.
    #[test]
    fn a_delivered_resolution_outranks_an_expired_backstop() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 5);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 5,
            resolution: LinkResolution::Ready,
        })
        .expect("배달");

        let verdict = manager.link_activation_verdict(id, 5, &rx, Duration::ZERO);
        assert!(
            matches!(verdict, EarlyVerdict::Ready),
            "시한이 배달을 이겼다 — 막 성립한 연결이 실패로 기록되고 죽는다: got {verdict:?}"
        );
    }

    /// ★아무도 배달하지 않으면 백스톱이 포기한다★ — 그 결말은 성공이 아니다.
    #[test]
    fn nothing_delivered_ends_at_the_backstop_as_a_failure() {
        let manager = bare_manager();
        let (id, _kills) = link_epoch_fixture(&manager, 1);
        let (_tx, rx) = std::sync::mpsc::channel();

        let verdict = manager.link_activation_verdict(id, 1, &rx, Duration::from_millis(150));
        assert!(
            matches!(verdict, EarlyVerdict::LinkFailed { .. }),
            "연결이 결말을 못 냈는데 성공으로 떨어졌다 — got {verdict:?}"
        );
    }

    /// ★사용자 kill 은 **배달 없이** 종점으로 온다 — 그래서 이어받기 실패로 기록되지 않는다★.
    ///
    /// 통로가 kill 갈래를 배달하지 않는 것이 전제이고(그쪽 `deliver_link` 가 그 규율의 정본), 여기서는
    /// 그 전제 위에서 감독자가 무엇을 내는지 본다: 종점 갈래이고, 거기엔 「사용자가 끈 것은 실패가
    /// 아니다」 규율이 이미 있다.
    #[test]
    fn a_user_kill_reaches_the_verdict_as_terminal_not_as_a_link_failure() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (core, _kills) = put_session_with_link(&manager, id, 2);
        let (_tx, rx) = std::sync::mpsc::channel();
        core.finish(crate::types::TerminalReason::Killed);

        let verdict = manager.link_activation_verdict(id, 2, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "사용자 종료가 연결 실패로 읽혔다 — 그러면 사용자의 취소가 이어받기 실패로 기록되고 그 위에              정리까지 한 번 더 돈다: got {verdict:?}"
        );
    }

    /// ★★사용자 kill 이 배달을 **억제**하면 채널이 닫히는데, 그것을 실패로 읽지 않는다★★.
    ///
    /// 이 라운드가 되살릴 뻔한 결함이다: `deliver_link` 는 **메시지**를 억제하지만 **채널 소멸**은 못
    /// 막는다(포트를 쥔 라이터 스레드가 끝나면 보내는 끝이 떨어진다). 그때 `session.kill()` 은 아직
    /// `child.kill()`/`wait()` 안이라 pump 가 `finish` 를 못 돌았고, 상태는 `Exiting` — **종점이 아니다.**
    /// 상태만 보던 옛 모양은 그 한 번의 읽기로 「통로가 결말을 배달하지 못한 채 사라졌다」를 확정했고,
    /// 그래서 **사용자의 취소가 이어받기 실패로 기록됐다** — 억제 장치가 막으려던 바로 그 결말이다.
    /// ★시험대가 그 인터리빙을 **결정적으로** 만든다★: 의도 래치를 세우고 `Exiting` 으로 전이시킨 뒤
    ///   보내는 끝을 떨어뜨린다 — 실제 kill 경로가 통과하는 상태 그대로다(종점은 아직 안 섰다).
    #[test]
    fn a_kill_that_suppresses_delivery_is_not_read_as_a_link_failure() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, _kills) = put_session_with_link(&manager, id, 13);
        let session = manager.get_session(id).expect("전제: 세션이 명부에 있다");

        // kill_agent 가 통로를 내리기 전에 지나가는 두 줄 그대로.
        session.set_intent(TerminationIntent::UserKill);
        let _ = session.enter_exiting();
        assert!(
            !matches!(
                session.status(),
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ),
            "전제: 아직 종점이 아니다 — 이 항목이 재는 창이 바로 그 구간이다"
        );

        // 억제된 배달 = 아무 메시지 없이 보내는 끝이 떨어진다.
        let (tx, rx) = std::sync::mpsc::channel::<LinkVerdict>();
        drop(tx);

        let verdict = manager.link_activation_verdict(id, 13, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "사용자가 끊어서 닫힌 채널을 「연결 실패」로 읽었다 — 그러면 사용자의 취소가 활성화 실패로              기록되고, 억제 장치가 하는 일이 없어진다: got {verdict:?}"
        );
    }

    /// ★★큐에 선 `Ready` 가 사용자 취소를 **이기지 못한다**★★ — 위 형제의 부호 반대 판.
    ///
    /// 핸드셰이크가 성공해 `Ready` 가 배달된 직후 사용자가 끊으면, 배달을 먼저 받는 모양은 **죽은
    /// 에이전트를 성공으로 보고**한다(`restore_one` 은 `Resumed`, `activate_profile` 은 `Ok(info)`).
    /// ★그래도 「종점이 배달을 이긴다」로 넓히지는 않았다★ — 스스로 죽는 것은 ADR-0201 이 「별개 사건」
    ///   으로 못박았고, 그쪽은 판정을 뒤집는 대신 호출자가 상태를 다시 떠서 돌려준다. 이기는 것은
    ///   **사람이 정한 결말** 하나뿐이다.
    #[test]
    fn a_queued_ready_does_not_outrank_a_user_cancellation() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, _kills) = put_session_with_link(&manager, id, 21);
        let session = manager.get_session(id).expect("전제: 세션이 명부에 있다");

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 21,
            resolution: LinkResolution::Ready,
        })
        .expect("배달");
        // 배달이 큐에 선 **뒤** 사용자가 끊는다.
        session.set_intent(TerminationIntent::UserKill);
        let _ = session.enter_exiting();

        let verdict = manager.link_activation_verdict(id, 21, &rx, Duration::from_secs(6));
        assert!(
            matches!(
                verdict,
                EarlyVerdict::Terminal {
                    status: AgentStatus::Killed,
                    ..
                }
            ),
            "큐에 선 준비 신호가 사용자 취소를 이겼다 — 방금 사용자가 끈 에이전트를 성공으로 보고한다:              got {verdict:?}"
        );
    }

    /// ★취소가 가로챈 배달의 **사유는 증거로 살아남는다**★ — 그 문구는 두 꼬리 어디에도 없다.
    #[test]
    fn a_cancellation_keeps_the_reason_of_the_delivery_it_intercepted() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, _kills) = put_session_with_link(&manager, id, 33);
        let session = manager.get_session(id).expect("전제: 세션이 명부에 있다");

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 33,
            resolution: LinkResolution::Failed {
                reason: "thread/resume: -32600 no rollout found for thread id".into(),
            },
        })
        .expect("배달");
        session.set_intent(TerminationIntent::UserKill);

        let EarlyVerdict::Terminal { evidence, .. } =
            manager.link_activation_verdict(id, 33, &rx, Duration::from_secs(6))
        else {
            panic!("전제: 취소는 종점 갈래로 떨어진다");
        };
        assert!(
            evidence.contains("-32600"),
            "가로챈 배달의 사유가 사라졌다 — 그 문구는 콘솔에도 진단에도 없어서 여기서 안 실으면              어디에도 안 남는다: {evidence:?}"
        );
    }

    /// ★★Fresh 성공은 **기다린 뒤의 상태**를 돌려준다 — spawn 시점 스냅샷이 아니다★★.
    ///
    /// `info` 는 자식이 뜬 직후 조립되고 status 가 `Running` 이다. 예전엔 0ms 된 값이라 그대로 내도
    /// 무해했지만, 연결을 기다리는 지금은 최대 백스톱만큼 낡는다 — 그 사이 죽은 에이전트를 호출자가
    /// 산 것으로 보고받으면(WS `Spawned` 가 그 status 를 그대로 나른다) **다음 동사가 시체에게 편지를
    /// 쓴다.** Resume 형제는 `activate_profile` 에서 같은 일을 한다.
    /// ★왜 소스에서 재나★: 이 갈래를 실행으로 몰려면 실 codex 가 핸드셰이크를 성공시켜 줘야 한다.
    #[test]
    fn a_fresh_success_reports_the_state_after_the_wait_not_the_spawn_snapshot() {
        let ready = fresh_arm("EarlyVerdict::Ready =>", "EarlyVerdict::LinkFailed");
        assert!(
            ready.contains("self.agent_info_by_id(profile.id)"),
            "성공 갈래가 spawn 시점 스냅샷을 그대로 돌려준다 — 백스톱만큼 낡은 `Running` 이 호출자에게              간다: {ready}"
        );
        assert!(
            ready.contains("AgentStatus::Exited { code: None }"),
            "조회가 놓쳤을 때 상태를 낮추지 않는다 — 그러면 수거된 세션이 산 것으로 보고된다: {ready}"
        );
    }

    /// ★★실패한 활성화의 정리는 **그 화신에만** 닿는다★★ — 결함 ①.
    ///
    /// 정리는 판정만큼 늦게 도착하고, 그 사이 이 화신이 수거되고 다음 화신이 떴을 수 있다. 표식을 안 보면
    /// 그때 죽는 것은 실패한 세션이 아니라 **막 뜬 건강한 후임**이다.
    #[test]
    fn a_stale_teardown_never_touches_a_live_successor() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (_core, kills) = put_session_with_link(&manager, id, 42);

        // 죽은 화신(41)의 정리가 뒤늦게 도착한다 — 지금 명부에 있는 것은 42 다.
        manager.tear_down_failed_activation(id, 41);
        assert_eq!(
            *kills.lock().expect("shutdowns poisoned"),
            0,
            "죽은 화신의 정리가 산 후임을 죽였다"
        );

        // 자기 화신의 정리는 그대로 돈다(위 단언만 있으면 「아무것도 안 한다」도 통과한다).
        manager.tear_down_failed_activation(id, 42);
        assert_eq!(
            *kills.lock().expect("shutdowns poisoned"),
            1,
            "자기 화신의 정리가 돌지 않았다 — 실패한 세션이 아무도 못 거두는 채로 남는다"
        );
    }

    /// ★★아직 결말이 안 난 활성화를 「도는 중」이라고 답하지 않는다★★ — 두 호출자가 갈리는 자리.
    ///
    /// 연결을 선언하는 통로에서는 세션이 명부에 오른 뒤에도 그 화신이 입력을 받을 수 있는지가 아직
    /// 안 정해져 있다. 그 구간에 들어온 두 번째 활성화가 `Ok`(도는 중)를 받으면, 첫 요청이 곧 실패로
    /// 판정해 그 화신을 거두는 순간 **두 호출자가 서로 다른 사실을 들고 갈린다**.
    /// ★예약을 직접 들고 그 구간을 만든다★ — 스레드 경쟁을 기다리지 않는다(`LinkWatch` 가 실물에서
    ///   하는 일이 정확히 이것이다: 결말이 날 때까지 예약을 놓지 않는다).
    /// ★대조군이 요점이다★ — 예약이 없을 때는 같은 조합이 여전히 「도는 중」으로 답해야 한다. 그 단언이
    ///   없으면 「항상 거절한다」도 통과하고, 그건 ADR-0082 의 재활성화 가드를 부수는 것이다.
    #[test]
    fn an_unresolved_activation_is_not_reported_to_a_second_caller_as_running() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            "C:/still-establishing",
            Some("still-establishing"),
        );
        let id = profile.id;
        // 첫 요청이 명부에 올려 둔 화신.
        let (_core, _kills) = put_session_with_link(&manager, id, 4);

        let settled = manager.activate_profile(&profile, SpawnMode::Fresh);
        assert_eq!(
            settled.as_ref().ok().map(|i| i.id),
            Some(id),
            "전제: 결말이 난 화신은 그대로 「도는 중」으로 답한다(ADR-0082 재활성화 가드)"
        );

        // 첫 요청이 결말을 기다리는 그 구간 — `LinkWatch` 가 예약을 쥐고 있다.
        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("예약");
        let during = manager.activate_profile(&profile, SpawnMode::Fresh);
        assert!(
            during.is_err(),
            "연결이 아직 안 선 화신을 「도는 중」으로 답했다 — 첫 요청이 그것을 실패로 판정해 거두면 이              호출자는 자기 밑에서 죽을 에이전트를 산 것으로 들고 있게 된다: got {:?}",
            during.map(|i| i.id)
        );
    }

    /// ★같은 구간에서 `spawn_agent` 도 「떠 있다」로 답하지 않는다★ — 예약을 명부 조회보다 **먼저**
    /// 보는 그 순서가 여기서 잡힌다.
    #[test]
    fn a_spawn_request_during_an_unresolved_activation_does_not_hand_back_the_live_session() {
        let manager = bare_manager();
        let profile = create(
            &manager,
            "C:/moot-vs-establishing",
            Some("moot-vs-establishing"),
        );
        let id = profile.id;
        let (_core, _kills) = put_session_with_link(&manager, id, 6);

        let settled = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("중복 요청은 오류가 아니다");
        assert_eq!(
            settled.into_info().map(|i| i.id),
            Some(id),
            "전제: 결말이 난 화신은 moot 이어도 그 정보를 돌려준다"
        );

        let _held = SpawnReservation::reserve(manager.spawning.clone(), id).expect("예약");
        let during = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("중복 요청은 오류가 아니다");
        assert!(
            during.into_info().is_none(),
            "연결이 아직 안 선 화신을 「떠 있다」로 돌려줬다 — 명부 조회가 예약보다 앞서면 이 답이 나온다"
        );
    }

    /// ★위 두 항목이 **못 재는 절반**을 여기서 잰다★ — 그 구간이 실제로 **연결의 결말까지** 이어지나.
    ///
    /// 위 둘은 예약을 시험대가 직접 들고 그 구간을 모사하므로, 운영 코드가 예약을 spawn 이 끝나는
    /// 자리에서 놓아 버려도 **초록으로 남는다**. 그러면 가드는 그대로인데 지켜야 할 구간이 사라진다.
    /// ★왜 소스에서 재나★: 그 구간을 실행으로 관측하려면 실 codex app-server 가 핸드셰이크를 붙들고
    ///   있어 줘야 한다 — 시험대에 그 상대가 없다.
    /// ★이 항목이 **못 잡는 것**도 적어 둔다★: 결말을 기다리는 쪽이 받은 값에서 배달함만 꺼내 들고
    ///   나머지를 버리면(예: `watch.map(|w| w.rx)`) 예약은 조용히 풀리고 이 단언은 그대로 통과한다.
    ///   그 모양을 막는 것은 컴파일러도 이 항목도 아니고, 두 호출자가 `watch` 를 통째로 들고 있다는
    ///   사실뿐이다.
    #[test]
    fn the_link_watch_holds_the_spawn_reservation_until_the_verdict() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");

        let declaration = production
            .split("struct LinkWatch {")
            .nth(1)
            .expect("`LinkWatch` 선언")
            .split('}')
            .next()
            .expect("선언 본문");
        assert!(
            declaration.contains("_reservation: SpawnReservation"),
            "배달함이 예약을 더는 들고 있지 않다 — 그러면 세션이 명부에 오른 순간 예약이 풀리고, 연결이              서는 동안 들어온 두 번째 요청이 다시 「떠 있다」로 답한다: {declaration}"
        );

        let spawning = production
            .split("fn spawn_agent_watching_link(")
            .nth(1)
            .expect("`spawn_agent_watching_link` 본문")
            .split("pub fn activate_profile(")
            .next()
            .expect("다음 함수까지");
        assert!(
            spawning.contains("_reservation: reservation"),
            "예약이 배달함에 실리지 않는다 — 이 함수가 돌아오는 순간 풀린다: {spawning}"
        );
    }

    /// 운영 구획에서 `spawn_fresh_settled` 의 한 갈래 본문만 잘라 온다.
    fn fresh_arm(start: &str, end: &str) -> String {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split("fn spawn_fresh_settled(")
            .nth(1)
            .expect("`spawn_fresh_settled` 본문")
            .split("fn spawn_session(")
            .next()
            .expect("다음 함수까지");
        let i = body
            .find(start)
            .unwrap_or_else(|| panic!("`{start}` 갈래가 없다 — 이 항목의 전제가 낡았다"));
        let rest = &body[i..];
        let j = rest
            .find(end)
            .unwrap_or_else(|| panic!("`{end}` 가 그 뒤에 없다 — 이 항목의 전제가 낡았다"));
        rest[..j].to_string()
    }

    /// ★★Fresh 스폰도 연결의 결말을 보고 나서 성공을 말한다★★ — 그리고 **두 입구 다** 그렇게 한다.
    ///
    /// ★왜 소스에서 재나★: 이 갈래를 실행으로 몰려면 실 codex app-server 가 `thread/start` 를 거절해
    ///   줘야 하는데 시험대에 그 상대가 없다(`link_activation_verdict` 단위 항목들이 그래서 채널을 직접
    ///   쥔다). 회귀했을 때 나는 것은 빨간 단언이 아니라 **조용한 무주공산**이다 — 화면엔 Running 인데
    ///   입력은 전부 거절되고 아무도 거두지 않는다.
    /// ★입구를 둘 다 재는 것이 요점이다★ — 하나만 고치면 다른 하나가 뒤처지고, 부팅 복원 쪽이 뒤처지면
    ///   그 구멍이 이어받을 손잡이 없는 codex 프로필 수만큼 한꺼번에 선다.
    #[test]
    fn both_fresh_entrances_settle_the_link_before_reporting_success() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");

        let settling = production
            .split("fn spawn_fresh_settled(")
            .nth(1)
            .expect("`spawn_fresh_settled` 본문")
            .split("fn spawn_session(")
            .next()
            .expect("다음 함수까지");
        assert!(
            settling.contains("self.link_activation_verdict("),
            "Fresh 갈래가 연결의 결말을 보지 않는다 — 거절당한 핸드셰이크에 주인이 없어진다: {settling}"
        );
        assert!(
            settling.contains("self.note_spawn_result("),
            "연결 축이 없는 통로(claude·shell)의 옛 기록 경로가 사라졌다 — 그 경로는 이 변경의 범위 밖이다"
        );

        let activate = production
            .split("pub fn activate_profile(")
            .nth(1)
            .expect("`activate_profile` 본문")
            .split("fn note_activation_result(")
            .next()
            .expect("다음 함수까지");
        assert!(
            activate.contains("self.spawn_fresh_settled(profile)"),
            "수동 활성화의 Fresh 갈래가 판정을 안 도는 동사로 돌아갔다: {activate}"
        );

        let restore = production
            .split("fn restore_one(")
            .nth(1)
            .expect("`restore_one` 본문")
            .split("fn resume_no_fallback(")
            .next()
            .expect("다음 함수까지");
        assert!(
            restore.contains("self.spawn_fresh_settled(profile)"),
            "부팅 복원의 비-이어받기 갈래가 판정을 안 도는 동사로 돌아갔다: {restore}"
        );

        assert_eq!(
            production.matches("self.note_spawn_result(").count(),
            1,
            "「spawn 한 번으로 끝나는 기록」의 호출자가 늘었다 — 그 동사는 연결 축이 **없는** 통로 전용이고,              축이 있는 곳에서 부르면 그것이 곧 ADR-0202 가 막는 증거 파괴다"
        );
    }

    /// ★Fresh 의 낙관적 성공도 「마지막 실패」를 지우지 않는다★(ADR-0202) — Resume 쪽 형제와 같은 규율.
    ///
    /// ADR-0202 가 「아직 참이 아니다」로 적어 둔 구멍이 정확히 이 자리였다: Fresh 는 프로세스가 뜬
    /// 순간을 성공으로 읽고 앞선 실패 기록을 지웠다.
    #[test]
    fn a_fresh_success_does_not_clear_the_failure_record() {
        let ready = fresh_arm("EarlyVerdict::Ready =>", "EarlyVerdict::LinkFailed");
        assert!(
            !ready.contains("note_activation_result"),
            "Fresh 성공 갈래가 「마지막 실패」를 건드린다 — 이 판정은 「입력을 받을 준비가 됐다」까지만              관측하므로, 그것으로 옛 실패 증거를 지우면 되돌릴 수 없는 소실이다: {ready}"
        );
    }

    /// ★Fresh 실패 갈래도 **정리를 먼저, 기록을 나중에**★ — Resume 쪽 형제와 같은 순서·같은 사유.
    ///
    /// 기록은 프로필 뮤텍스를 잡는데 그 뮤텍스가 붙들려 있으면, 순서가 뒤집힌 쪽은 **자식을 영영 안
    /// 죽인다**. 그리고 정리는 화신 표식을 들고 불려야 한다 — 안 그러면 산 후임을 죽인다.
    #[test]
    fn a_failed_fresh_link_tears_down_before_it_records() {
        let arm = fresh_arm(
            "EarlyVerdict::LinkFailed { reason } => {",
            "EarlyVerdict::Terminal {",
        );
        let teardown = arm
            .find("tear_down_failed_activation")
            .expect("Fresh 연결 실패 갈래가 정리를 부르지 않는다 — 실패한 세션을 아무도 못 거둔다");
        let record = arm
            .find("note_activation_result")
            .expect("Fresh 연결 실패 갈래가 실패를 기록하지 않는다 — 증거가 어디에도 안 남는다");
        assert!(
            teardown < record,
            "기록이 정리보다 앞선다 — 프로필 뮤텍스가 붙들려 있으면 자식이 영영 안 죽는다: {arm}"
        );
        assert!(
            arm.contains("tear_down_failed_activation(profile.id, incarnation)"),
            "정리가 화신 표식 없이 불린다 — 그 사이 다른 화신이 섰으면 산 후임을 죽인다: {arm}"
        );
    }

    /// 운영 구획에서 `resume_no_fallback` 의 한 갈래 본문만 잘라 온다.
    fn resume_arm(start: &str, end: &str) -> String {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split("fn resume_no_fallback(")
            .nth(1)
            .expect("`resume_no_fallback` 본문")
            .split("fn tear_down_failed_activation(")
            .next()
            .expect("다음 함수까지");
        let i = body
            .find(start)
            .unwrap_or_else(|| panic!("`{start}` 갈래가 없다 — 이 항목의 전제가 낡았다"));
        let rest = &body[i..];
        let j = rest
            .find(end)
            .unwrap_or_else(|| panic!("`{end}` 가 그 뒤에 없다 — 이 항목의 전제가 낡았다"));
        rest[..j].to_string()
    }

    /// ★낙관적 성공은 「마지막 실패」를 **지우지 않는다**★(사용자 조건) — 원래 결함의 절반이 그 파괴였다.
    ///
    /// ★왜 소스에서 재나★: 지움은 `note_activation_result(.., None)` 호출 **하나**이고, 그 부재를
    ///   실행으로 재려면 실 codex 로 이어받기를 성공시켜 놓고 옛 실패 기록이 남아 있는지 봐야 한다 —
    ///   시험대에 그 조합이 없다. 회귀했을 때 나는 것은 빨간 단언이 아니라 **조용한 증거 소실**이다.
    /// ★`Alive` 갈래에는 그 호출이 **남아 있어야** 한다★ — 그쪽은 연결 축이 없는 통로(claude)의 길이고
    ///   이 결정의 범위 밖이다. 둘을 함께 재야 「범위를 안 넘었다」까지 잡힌다.
    #[test]
    fn an_optimistic_success_does_not_clear_the_failure_record() {
        let ready = resume_arm("EarlyVerdict::Ready => {", "EarlyVerdict::Alive => {");
        assert!(
            !ready.contains("note_activation_result"),
            "낙관적 성공 갈래가 「마지막 실패」를 건드린다 — 이 판정은 「입력을 받을 준비가 됐다」까지만              관측하므로, 그것으로 옛 실패 증거를 지우면 되돌릴 수 없는 소실이다: {ready}"
        );

        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");
        let body = production
            .split("fn resume_no_fallback(")
            .nth(1)
            .expect("본문");
        let alive = &body[body.find("EarlyVerdict::Alive => {").expect("`Alive` 갈래")..];
        assert!(
            alive.contains("note_activation_result(profile.id, Some(spawned.epoch), None)"),
            "연결 축이 없는 통로(claude)의 지움까지 함께 걷혔다 — 이 결정의 범위는 연결을 선언하는              통로뿐이다"
        );
    }

    /// ★실패 갈래는 **정리를 먼저, 기록을 나중에** 한다★ — 순서가 뒤집히면 백스톱이 무력해진다.
    ///
    /// 기록은 프로필 뮤텍스를 잡는데, 그 뮤텍스는 세션 id 기록 경로가 디스크 쓰기를 쥔 채 들고 있을 수
    /// 있다. 기록이 앞서면 그 정체가 곧 **「자식을 영영 안 죽인다」**가 된다 — 백스톱은 바로 그런 정체를
    /// 위해 있는데, 그 정체 때문에 못 쓰게 되는 모양이다.
    /// ★그리고 정리는 **화신 표식을 들고** 불려야 한다★ — 안 그러면 산 후임을 죽인다(결함 ①).
    #[test]
    fn the_failed_activation_arm_tears_down_before_it_records() {
        let arm = resume_arm(
            "EarlyVerdict::LinkFailed { reason } => {",
            "EarlyVerdict::Ready => {",
        );
        let teardown = arm
            .find("tear_down_failed_activation")
            .expect("연결 실패 갈래가 정리를 부르지 않는다 — 실패한 세션을 아무도 못 거둔다");
        let record = arm
            .find("note_activation_result")
            .expect("연결 실패 갈래가 실패를 기록하지 않는다");
        assert!(
            teardown < record,
            "기록이 정리보다 앞선다 — 프로필 뮤텍스가 붙들려 있으면 자식이 영영 안 죽는다: {arm}"
        );
        assert!(
            arm.contains("tear_down_failed_activation(profile.id, spawned.epoch)"),
            "정리가 화신 표식 없이 불린다 — 그 사이 다른 화신이 섰으면 산 후임을 죽인다: {arm}"
        );
    }

    /// ★종점이 배달을 앞질러도 **사유는 살아남는다**★ — 이 통로의 실패 문구는 두 꼬리 어디에도 없다.
    ///
    /// 실 거절 경로에서 상대는 41–51ms 만에 exit 하므로 종점 관측이 배달보다 먼저 들 수 있다. 그때
    /// 사유를 버리면 원인 대신 맥락 기본값(조기 종료)이 마지막 실패에 찍힌다.
    #[test]
    fn a_terminal_verdict_still_carries_the_delivered_reason() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let (core, _kills) = put_session_with_link(&manager, id, 9);
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(LinkVerdict {
            epoch: 9,
            resolution: LinkResolution::Failed {
                reason: "thread/resume: -32600 no rollout found for thread id".into(),
            },
        })
        .expect("배달");
        // 두 꼬리는 **빈 채로** 둔다 — 구조화 통로의 실제 모양(거절은 stdout 으로 온다).
        core.finish(crate::types::TerminalReason::Exited { code: Some(1) });

        // ★어느 갈래로 떨어지든 **사유는 살아 있어야 한다**★ — 배달과 종점 관측 중 무엇이 먼저 들지는
        //   타이밍이고, 그것을 못 박으면 항목 자체가 깜빡이가 된다. 재는 것은 「원인이 분류에 닿나」다.
        let verdict = manager.link_activation_verdict(id, 9, &rx, Duration::from_millis(300));
        let carried = match &verdict {
            EarlyVerdict::LinkFailed { reason } => reason.clone(),
            EarlyVerdict::Terminal { evidence, .. } => evidence.clone(),
            other => panic!("전제: 실패로 관측된다 — got {other:?}"),
        };
        assert_eq!(
            backend::resume_failure_kind(&codex_app_server_command(), &carried),
            Some(AgentFailureKind::NoConversationToResume),
            "배달된 사유가 판정 밖으로 나오지 못했다 — 원인 대신 맥락 기본값(조기 종료)이 마지막 실패에              찍힌다: verdict={verdict:?}"
        );
    }

    fn codex_app_server_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Codex {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        }
    }

    fn claude_terminal_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::Terminal,
        }
    }

    #[cfg(windows)]
    fn claude_stream_json_command() -> crate::profile::AgentCommand {
        crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        }
    }

    // ── ADR-0172: 판정은 죽음이 아니라 **증거**로도 선다 ────────────────────────────────

    /// ★이 기능의 표제 사례 회귀망(실측 2026-08-23)★ — 진단이 이미 실패를 말했으면 프로세스가 아직
    /// 살아 있어도 그 자리에서 확정한다.
    ///
    /// 잡는 회귀는 정확히 이것이다: 실 claude 는 `--resume <빈 sid>` 에 대해 **+2.2s 에 진단을 내고
    /// +6.4s 에야 죽는다**(그 사이 SessionEnd 훅이 돈다). 죽음만 기다리는 옛 판정은 3s 창을 넘겨
    /// **실패를 성공으로 판정하고 앞선 기록까지 지웠다** — 표제 사례가 통째로 죽어 있었다.
    ///
    /// ★시계를 단언하지 않는다(이 저장소의 옛 flaky 재발 방지)★: "빨리 돌아왔다" 를 경과시간으로 재지
    ///   않는다. 대신 **창을 아주 길게** 주고 결말만 본다 — 죽음을 기다리는 구현이라면 이 세션은 스스로
    ///   죽지 않으므로 창 끝까지 갔다가 `Alive` 를 내고, 그 단언이 붉어진다. 통과 조건이 타이밍에
    ///   좌우되지 않는다.
    #[test]
    fn a_diagnosed_failure_settles_the_verdict_without_waiting_for_the_process_to_die() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // 시드는 단언 밖에서 한다(행위-in-단언 회피).
        core.push_diagnostic("No conversation found with session ID: 8b1c");

        // ★창을 운영값(3s)보다 훨씬 길게 준다★: 이 세션은 스스로 죽지 않으므로, 죽음만 기다리는 구현은
        //   창 끝까지 갔다가 `Alive` 를 내고 아래 단언이 붉어진다. 값이 크면 "우연히 데드라인에 걸려
        //   통과" 가 불가능해진다(그 대가는 회귀 시 red 가 느린 것뿐이다).
        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_secs(30),
            LINK_RESOLUTION_BACKSTOP,
        );

        assert!(
            matches!(
                verdict,
                EarlyVerdict::Diagnosed(AgentFailureKind::NoConversationToResume)
            ),
            "진단이 이미 말했으면 시체를 기다리지 않는다 — got {:?}",
            match verdict {
                EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                EarlyVerdict::Alive => "Alive(죽음만 기다리는 옛 판정 = 회귀)".into(),
                EarlyVerdict::Terminal { ref status, .. } => format!("Terminal({status:?})"),
                EarlyVerdict::LinkFailed { ref reason } => format!("LinkFailed({reason})"),
                EarlyVerdict::Ready => "Ready".into(),
            }
        );
        assert!(
            !matches!(
                manager.get_session(id).expect("세션은 명부에 있다").status(),
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ),
            "판정 시점에 이 세션은 아직 살아 있어야 한다 — 죽었다면 이 테스트는 옛 갈래를 재는 것이다"
        );
    }

    /// ★같은 자리의 오탐 방어★ — 진단이 침묵하면 살아 있는 세션은 그대로 성립으로 넘어간다.
    ///
    /// 이 단언이 없으면 위 테스트는 "아무 텍스트에나 Diagnosed 를 내는" 구현으로도 통과한다. 그 구현은
    /// **성공한 이어받기를 실패로 도장 찍는다**(fail-open 위반 — ADR-0172 §영향).
    #[test]
    fn a_live_session_with_nothing_to_say_is_not_diagnosed() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // 시드는 단언 밖에서 한다 — 모르는 문구는 단정 대상이 아니다.
        core.push_diagnostic("[DEBUG] loading plugins…");

        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_millis(250),
            LINK_RESOLUTION_BACKSTOP,
        );

        assert!(
            matches!(verdict, EarlyVerdict::Alive),
            "모르는 진단은 활성화를 막지 않는다(fail-open) — got {}",
            match verdict {
                EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                EarlyVerdict::Alive => "Alive".into(),
                EarlyVerdict::Terminal { ref status, .. } => format!("Terminal({status:?})"),
                EarlyVerdict::LinkFailed { ref reason } => format!("LinkFailed({reason})"),
                EarlyVerdict::Ready => "Ready".into(),
            }
        );
    }

    /// ★죽은 세션의 증거에 **진단 스트림**도 들어간다★ — 구조화 세션은 콘솔 링이 비고 stderr 만 찬다.
    ///
    /// 이 갈래가 빠져 있었던 것이 결함 (2) 다: 분류기는 콘솔 꼬리만 봤고, stream-json 이 기본 출력
    /// 형식이라 **대부분의 에이전트에서 분류기가 눈이 먼 채로** 돌았다.
    #[test]
    fn a_dead_sessions_diagnostic_stream_counts_as_evidence_too() {
        let manager = bare_manager();
        let id = AgentId::new_v4();
        let core = put_live_session(&manager, id, 1);
        // 시드는 단언 밖에서 한다 — 콘솔 링은 **비운 채로** 둔다(구조화 세션의 실제 모양).
        core.push_diagnostic("No conversation found with session ID: 8b1c");
        core.finish(TerminalReason::Exited { code: Some(1) });

        let verdict = manager.early_activation_verdict(
            id,
            &claude_terminal_command(),
            Duration::from_secs(5),
            LINK_RESOLUTION_BACKSTOP,
        );

        let evidence = match verdict {
            EarlyVerdict::Terminal { evidence, .. } => evidence,
            other => panic!(
                "전제: 종점 상태로 관측된다 — got {}",
                match other {
                    EarlyVerdict::Diagnosed(k) => format!("Diagnosed({k:?})"),
                    EarlyVerdict::Alive => "Alive".into(),
                    EarlyVerdict::LinkFailed { reason } => format!("LinkFailed({reason})"),
                    EarlyVerdict::Ready => "Ready".into(),
                    EarlyVerdict::Terminal { .. } => unreachable!(),
                }
            ),
        };
        assert_eq!(
            backend::resume_failure_kind(&claude_terminal_command(), &evidence),
            Some(AgentFailureKind::NoConversationToResume),
            "콘솔 링이 비어도 stderr 가 분류를 세워야 한다 — got evidence {evidence:?}"
        );
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
                _frame: crate::types::OutputFrame<'_>,
            ) -> Result<(), crate::types::SinkError> {
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
            crate::profile::AgentCommand::Shell {
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
        impl crate::profile::ProfileStore for MemStore {
            fn save(&self, profiles: &[AgentProfile]) {
                *self.0.lock().expect("mem store poisoned") = profiles.to_vec();
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.lock().expect("mem store poisoned").clone()
            }
        }
        let tag = uuid::Uuid::new_v4();
        let profiles = Arc::new(crate::profile::ProfileRegistry::new(Arc::new(
            MemStore::default(),
        )));
        let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
            std::env::temp_dir().join(format!("engram-cap-preset-{tag}")),
        ))));
        let tracker = Arc::new(SessionTracker::new(
            crate::session_tracker::TrackerConfig {
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
        let raw_base = crate::name::cwd_basename(&dotted);
        let canon_base = {
            let c = dunce::canonicalize(&dotted).expect("temp 디렉터리는 실재한다");
            crate::name::cwd_basename(&c.to_string_lossy())
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

    // ── ADR-0172: 중복 요청은 오류가 아니라 「할 일 없음」이고, 아무것도 바꾸지 않는다 ────────────

    /// ★이미 뜨는 중인 항목에 온 요청 — 예약을 잡아 두어 **결정적으로** 그 상황을 만든다★.
    ///
    /// 스레드 경쟁을 기다리지 않는다: 예약을 직접 들고 있으면 그 뒤 오는 요청은 반드시 진다.
    /// 확인하는 것 셋 — 오류가 아니라 결말이라는 것 · 호출자가 「내가 띄운 게 아니다」를 구분할 수 있다는 것 ·
    /// 그리고 **기록도 지움도 일어나지 않는다**는 것(앞서 기록해 둔 값이 그대로 남는다).
    #[test]
    fn a_request_that_arrives_while_another_spawn_is_in_flight_is_moot_and_changes_nothing() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(&manager, "C:/moot-in-flight", Some("moot-in-flight"));
        let id = profile.id;
        // 시드는 단언 밖에서 한다(행위-in-단언 회피).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::NoConversationToResume),
        );
        assert!(seeded, "전제: 시드가 실제로 값을 바꾼다");

        // 다른 요청이 진행 중인 상황 그 자체 — 놓지 않고 들고 있는다.
        let _held =
            SpawnReservation::reserve(manager.spawning.clone(), id).expect("예약을 잡을 수 있다");

        let outcome = manager
            .activate_profile(&profile, SpawnMode::Fresh)
            .err()
            .map(|e| e.to_string());
        assert!(
            outcome.is_some(),
            "승자가 아직 명부에 올리기 전이라 돌려줄 세션이 없다 — NotFound 가 정직한 답이다"
        );
        assert!(
            manager.list_agents().is_empty(),
            "이 요청은 프로세스를 띄우지 않았다"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::NoConversationToResume),
            "할 일이 없었으므로 기록하지도, 지우지도 않는다"
        );
    }

    /// ★이미 떠 있는 항목에 온 요청 — 실 프로세스로 본다★. 위 형제와 갈리는 지점은 **돌려줄 세션이
    /// 있다**는 것이라, 호출자가 moot 을 받고도 그 에이전트 정보를 얻는다.
    #[cfg(windows)]
    #[test]
    fn a_request_for_an_already_running_agent_is_moot_and_returns_the_live_session() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("moot-live"),
        );
        let id = profile.id;
        let first = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("최초 spawn");
        assert!(first.started(), "전제: 첫 요청이 실제로 띄웠다");

        // 산 에이전트에 기록을 심어 두고, 중복 요청이 그것을 건드리지 않는지 본다.
        // 시드는 단언 밖에서 한다(행위-in-단언 회피).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::EarlyExitAfterResume),
        );
        assert!(seeded, "전제: 시드가 실제로 값을 바꾼다");

        let again = manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("중복 요청은 오류가 아니다");
        assert!(!again.started(), "이 호출은 아무것도 띄우지 않았다");
        assert_eq!(
            again.into_info().map(|i| i.id),
            Some(id),
            "moot 이어도 산 세션 정보는 돌려준다"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::EarlyExitAfterResume),
            "할 일이 없었으므로 기록도 지움도 없다"
        );
        assert_eq!(manager.list_agents().len(), 1, "화신은 여전히 하나다");

        manager.kill_agent(id).ok();
    }

    /// 활성화가 성립하면 그 자리에서 지운다(ADR-0172 결정 4 개정판) — 그리고 그 뒤에도 축은 여전히
    /// 별개다: 도는 항목에 기록을 다시 붙일 수 있어야 화면의 「도는 중이 이긴다」 규칙이 설 자리가 있다.
    ///
    /// ★in-crate 테스트인 이유★: 시드에 `set_last_failure` 가 필요하고 그 동사는 `pub(crate)` 다
    ///   (단일 쓰기 지점 경계를 crate 밖으로는 컴파일러가 지키게 한다).
    #[cfg(windows)]
    #[test]
    fn a_successful_activation_clears_the_last_failure_but_the_axis_stays_separate() {
        use crate::failure::AgentFailureKind;

        let manager = bare_manager();
        let profile = create(
            &manager,
            &std::env::temp_dir().to_string_lossy(),
            Some("clear-on-success"),
        );
        let id = profile.id;
        // 시드는 단언 밖에서 한다(행위-in-단언 회피).
        let seeded = manager.profiles.set_last_failure(
            id,
            None,
            Some(AgentFailureKind::NoConversationToResume),
        );
        assert!(seeded, "전제: 시드가 실제로 값을 바꾼다");

        let info = manager
            .activate_profile(&profile, SpawnMode::Fresh)
            .expect("활성화는 성공한다");
        assert!(
            !matches!(
                info.status,
                AgentStatus::Failed { .. } | AgentStatus::Killed | AgentStatus::Exited { .. }
            ),
            "전제: 이 세션은 살아 있다 — got {:?}",
            info.status
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            None,
            "활성화가 성립했으니 지워야 한다"
        );

        // 축 독립성 — 상태 열거형에 합쳤다면 이 조합이 아예 표현되지 않는다.
        // 시드는 단언 밖에서 한다(행위-in-단언 회피).
        let seeded = manager.profiles.set_last_failure(
            id,
            Some(info.epoch),
            Some(AgentFailureKind::SpawnFailed),
        );
        assert!(seeded, "전제: 시드가 실제로 값을 바꾼다");
        assert!(
            manager.list_agents().iter().any(|a| a.id == id),
            "여전히 산 명부에 있다"
        );
        assert_eq!(
            manager.agent_snapshot(id).and_then(|p| p.last_failure),
            Some(AgentFailureKind::SpawnFailed),
            "도는 중 + 마지막 실패 조합이 표현된다"
        );

        manager.kill_agent(id).ok();
    }

    /// ★죽은 세션의 출력 꼬리가 실제로 읽히는지 — 실 프로세스로만 말할 수 있다(ADR-0172)★.
    ///
    /// 이 테스트가 잡는 회귀는 하나다: 폴링 루프 **안에서** 세션을 다시 조회하면, `finish` 가 상태를 세운
    /// 직후 reaper 가 명부에서 지우므로(우리 폴링 간격은 100ms) 거의 언제나 조회가 실패해 꼬리가 빈다.
    /// 그러면 분류는 항상 맥락 기본값으로 떨어지고 「이어받을 대화 없음」 판정이 **죽은 코드**가 된다 —
    /// 순수 dispatch 테스트는 그 사실을 못 본다.
    #[cfg(windows)]
    #[test]
    fn early_verdict_captures_the_dead_sessions_output_tail() {
        use crate::failure::AgentFailureKind;

        // ★배치 파일인 이유★: portable-pty CommandBuilder 가 `&`·`>` 를 개별 quoting 해 ConPTY 통과 중
        //   깨뜨린다(activation.rs 픽스처의 실측). 배치는 cmd 가 직접 파싱하니 결정적이다.
        // ★죽기 전에 잠깐 산다(ping)★: 이 테스트의 대상은 "reaper 가 명부에서 지운 **뒤에도** 링을
        //   읽는가" 다. 즉사시키면 첫 `get_session` 이 reaper 와 경쟁해 세션을 못 잡는 쪽으로 질 수 있고,
        //   그러면 대상과 무관한 이유로 붉어진다(거짓 red). 잠깐 살려 Arc 획득을 확실히 한 뒤 죽인다 —
        //   수거 후 읽기라는 검증 대상은 그대로다.
        let uniq = uuid::Uuid::new_v4();
        let batch = std::env::temp_dir().join(format!("engram-tail-probe-{uniq}.cmd"));
        std::fs::write(
            &batch,
            "@echo off\r\necho No conversation found with session ID: 8b1c\r\nping -n 2 127.0.0.1 >nul\r\nexit /b 1\r\n",
        )
        .expect("배치 write");

        let manager = bare_manager();
        let mut draft = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("tail-probe"));
        draft.command = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), batch.to_string_lossy().to_string()],
        };
        let profile = manager.create_agent(draft).expect("등록");
        manager
            .spawn_agent(&profile, SpawnMode::Fresh)
            .expect("spawn")
            .into_started()
            .expect("이 호출은 실제로 띄운다(중복 요청 아님)");

        // ★shell 프로필이라 진단 버퍼는 비어 있다(PTY 는 stderr 를 콘솔에 병합한다)★ — 즉 여기서
        //   서는 증거는 오롯이 **콘솔 꼬리**이고, 그게 이 테스트가 재는 것이다.
        let EarlyVerdict::Terminal { status, evidence } = manager.early_activation_verdict(
            profile.id,
            &claude_terminal_command(),
            Duration::from_secs(10),
            LINK_RESOLUTION_BACKSTOP,
        ) else {
            panic!("이 배치는 즉시 죽는다(전제)")
        };
        assert!(
            matches!(
                status,
                AgentStatus::Exited { .. } | AgentStatus::Failed { .. } | AgentStatus::Killed
            ),
            "전제: 종점 상태로 관측된다 — got {status:?}"
        );
        assert!(
            evidence.contains("No conversation found"),
            "reaper 가 세션을 거둔 뒤에도 꼬리가 읽혀야 한다 — 비면 분류가 언제나 맥락 기본값이다. got {evidence:?}"
        );
        // 그 꼬리가 실제로 분류까지 이어진다(claude 어휘 — 실 claude 없이 dispatch 만 태운다).
        assert_eq!(
            backend::resume_failure_kind(&claude_terminal_command(), &evidence),
            Some(AgentFailureKind::NoConversationToResume),
            "실 세션에서 읽은 꼬리가 「이어받을 대화 없음」으로 분류돼야 한다"
        );

        let _ = std::fs::remove_file(&batch);
    }

    /// ★`spawn_agent` 안의 **세 동사 순서**를 못 박는다 — 이 순서가 이어받기 값의 레이스 없음을 낸다★.
    ///
    /// 못 박는 것 둘:
    ///   1. `register_for_spawn` **바로 뒤**가 `epoch_for_spawn` 이다. 앞엣것은 live 표식을 **보존**하므로
    ///      (ADR-0084) 그 사이 구간에는 여전히 **앞 화신의 표식**이 서 있고, 그 구간에 도착한 앞 화신의
    ///      지각 기록은 표식이 일치해 **거절되지 않는다**. 사이에 cwd 정규화(syscall)나 sid 발급
    ///      (`agents.json` 통째 쓰기)이 끼면 그 구간이 실제로 벌어진다 — 옛 배치가 그랬다.
    ///   2. 이어받기 손잡이를 읽는 자리가 `epoch_for_spawn` **뒤**다. 표식이 바뀐 뒤부터 이 spawn 이
    ///      기록 포트를 건네기 전까지는 어떤 화신의 기록도 통과하지 못해 명부 값이 얼어 있고, 그래서
    ///      「읽은 값 = 이 화신이 이어받는 값」이 성립한다.
    ///
    /// ★그리고 그 읽기가 **명부**여야 한다★ — 호출자 스냅샷(`profile.backend_session_id`)을 읽으면 그
    ///   사이 상대가 준 thread id 를 통째로 놓친다(그 칸은 우리가 아니라 상대가 쓴다).
    ///
    /// ★왜 소스에서 재나★: 이 순서를 실행으로 재려면 실 codex 바이너리와 지각 기록을 끼워 넣을 창이
    ///   동시에 있어야 한다. 그 둘은 시험대에 없고, 어긋났을 때 나는 것은 컴파일 에러도 빨간 단언도 아닌
    ///   **낮은 확률의 조용한 오기록**이다. 선례·같은 사유 =
    ///   `backend::codex::transport::tests::the_session_id_is_recorded_before_the_gate_opens`.
    #[test]
    fn the_resume_handle_is_read_from_the_roster_after_the_incarnation_tag_is_stamped() {
        let src = include_str!("manager.rs");
        let production = src.split("mod tests {").next().expect("운영 구획");

        // ★본문을 오려 내기 전에 **오려 낸 것이 맞는지** 먼저 본다★(리뷰 지적): 두 경계 이름은 자유
        //   텍스트라, 다른 함수가 `spawn_agent` 위로 올라가면 잘라 낸 덩어리가 엉뚱하게 커지고 아래
        //   순서 단언이 무관한 코드 위에서 초록이 된다. 그 조용한 통과를 막는 것이 이 두 줄이다.
        assert_eq!(
            production.matches("pub fn spawn_agent(").count(),
            1,
            "`spawn_agent` 이름이 운영 구획에 둘 이상이다 — 아래 오려내기가 어느 것을 잡았는지 알 수 없다"
        );
        let body = production
            .split("pub fn spawn_agent(")
            .nth(1)
            .expect("`spawn_agent` 본문")
            .split("pub fn activate_profile(")
            .next()
            .expect("다음 함수까지");
        assert!(
            !body.contains("pub fn "),
            "잘라 낸 덩어리에 다른 `pub fn` 이 들어 있다 — 경계 함수가 옮겨져 범위가 넘쳤다"
        );

        // ★바이트 오프셋이 아니라 **실행되는 줄의 나열**로 본다★ — rustfmt 가 체인을 줄로 쪼개므로
        //   한 덩어리 문자열 검색은 포매팅 한 번에 깨진다(실제로 깨졌다).
        let code: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .collect();
        let only = |needle: &str| -> usize {
            let hits: Vec<usize> = code
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(needle))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(
                hits.len(),
                1,
                "`{needle}` 이 `spawn_agent` 에서 {}회 잡힌다 — 정확히 하나여야 이 항목이 자리를 특정한다",
                hits.len()
            );
            hits[0]
        };

        let registered = only("self.register_for_spawn(profile)?;");
        let stamped = only(".epoch_for_spawn(profile.id)");
        let resume_read = only("let resume_session_id = match mode {");

        assert!(
            registered < stamped && stamped < resume_read,
            "세 동사의 순서가 어긋났다(등록 {registered} · 표식 {stamped} · 손잡이 읽기 {resume_read}) — \
             표식이 등록보다 앞서면 등록이 앞 화신 표식을 되살리고, 손잡이 읽기가 표식보다 앞서면 그 \
             구간의 명부 값이 앞 화신의 지각 기록에 열려 있다"
        );

        // 등록 **바로 다음 실행 줄**이 표식 바인딩의 시작이어야 한다(주석·빈 줄만 사이에 허용).
        assert!(
            code[registered + 1].starts_with("let epoch"),
            "`register_for_spawn` 다음 실행 줄이 표식 바인딩이 아니다 — 그만큼 앞 화신의 표식이 명부에 \
             서 있는 구간이 벌어지고, 그 구간에 도착한 지각 기록이 갓 발급한 sid 를 덮는다(옛 배치가 \
             canonicalize + `agents.json` 쓰기만큼 벌어져 있었다): {:?}",
            code[registered + 1]
        );

        let binding_end = code[resume_read..]
            .iter()
            .position(|l| *l == "};")
            .expect("이어받기 바인딩의 끝(`};`)");
        let binding = &code[resume_read..resume_read + binding_end];
        assert!(
            !binding.iter().any(|l| l.contains("profile.backend_session_id")),
            "이어받기 손잡이를 호출자 스냅샷에서 읽는다 — 스냅샷 뒤에 상대가 준 thread id 를 놓치고 \
             낡은 스레드로 이어받는다: {binding:?}"
        );
        assert!(
            binding.iter().any(|l| l.contains(".get(profile.id)")),
            "이어받기 손잡이를 명부에서 읽지 않는다: {binding:?}"
        );
        // ★부재를 삼키지 않는다★ — 프로필이 지워졌는데 `None` 으로 흘리면 이어받기가 말없이 새 대화가
        //   된다(위 그 줄 주석이 사유의 정본).
        assert!(
            binding.iter().any(|l| l.contains("profile_vanished_mid_spawn")),
            "프로필 부재를 끊지 않는다 — `and_then` 으로 삼키면 이어받기가 조용히 새 대화가 된다: {binding:?}"
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
            .expect("첫 spawn")
            .into_started()
            .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
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
            .expect("Fresh 재spawn")
            .into_started()
            .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
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
            .expect("기존 에이전트 spawn")
            .into_started()
            .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
        assert_eq!(
            first.name, "bob",
            "이미 명부에 있는 에이전트를 띄우는 건 개명 대상이 아니다"
        );

        let adhoc = agent_profile(&std::env::temp_dir().to_string_lossy(), Some("bob"));
        let second = manager
            .spawn_agent(&adhoc, SpawnMode::Fresh)
            .expect("ad-hoc spawn")
            .into_started()
            .expect("이 호출은 실제로 띄운다(중복 요청 아님)");
        assert_eq!(second.name, "bob(1)", "신규 등록 spawn 은 접미사를 받는다");

        manager.kill_agent(first.id).ok();
        manager.kill_agent(second.id).ok();
    }
}
