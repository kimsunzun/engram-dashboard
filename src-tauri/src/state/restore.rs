//! 비정상 종료 뒤 「복원할까요?」(TRD S21-storage §6-7) — 상태 셋(`none` · `awaiting` · `answered`)과 처리 중인 답
//! 하나의 표지, 그 실행의 `state.json` 읽기 결과([`StateFileStatus`] · §6-5)를 쥐는 복원 서비스([`RestoreService`]),
//! 그 위에서 답을 실행하는 복원 조율자([`RestoreCoordinator`]).
//! 상태는 부팅 단계 ⑥ 이 한 곳에서 정하고([`RestoreService::set_boot`] · I5), 답은 조율자의
//! [`RestoreCoordinator::answer`] 하나로 간다(사람 · LLM 같은 핸들).
//!
//! - ★서비스 락은 잎이다★ — 쥔 채 알림 · 로그 · 기록기 기다리기 · 다른 락(`ViewManager`)을 하지 않는다.
//!   알림([`RestoreNotifier`])은 락을 놓은 뒤 부른다.
//! - ★표지를 끝맺지 않고 버리면 `RolledBack` 과 같다★(패닉으로 풀린 경우 포함) — 처리 중 표지가 걸린 채 남으면
//!   뒤의 답이 모두 `InFlight` 를 받아 그 실행에서는 영영 답할 수 없다.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use ts_rs::TS;

use super::boot_plugin::{report_restore_warning, ResolveResult, StateSession};
use super::convert::{RestoreWarning, Restored};
use super::placement::{land, Fallback, Landing, MonitorArea};
use super::schema::{StateFile, WindowKind};
use crate::layout::manager::WindowLabel;
use crate::layout::{
    LabelSource, LayoutEvents, LayoutState, SubscriptionSync, ViewManager, ViewSnapshot,
    WindowAttrs, WindowBounds, WindowTabsPayload, MAIN_WINDOW_LABEL,
};
use crate::theme::ThemeControl;

/// 복원 상태가 바뀌면 main 창에 내는 사건 — 실을 것 = [`CrashCopyStatus`].
pub const EVT_RESTORE_CHANGED: &str = "restore:changed";

/// `restore.status` 의 `crash_copy` 칸이자 [`EVT_RESTORE_CHANGED`] 의 실을 것(TRD §6-7) — wire 철자는 `"none"` ·
/// `"awaiting"` · `"answered"`.
///
/// - `None` = 이 실행에서 물을 사본이 없다 — 이 판이 적용하지 못하는 사본(버전 초과 — N6)이 디스크에 있어도
///   이 값이다.
/// - `Awaiting` = 답하지 않은 사본이 있다 — 사람 UI 는 답할 때까지 main 을 막는다.
/// - `Answered` = 이 실행에서 답했다 — 사본 파일은 기록기가 지울 때까지 디스크에 남아 있을 수 있다(I2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CrashCopyStatus {
    None,
    Awaiting,
    Answered,
}

impl CrashCopyStatus {
    /// serde 철자 그대로 — 버스 `restore.status` 의 `crash_copy` 는 선언 매크로가 enum 을 소문자 철자로 못 실어
    /// 문자열이다.
    pub fn as_wire(self) -> &'static str {
        match self {
            CrashCopyStatus::None => "none",
            CrashCopyStatus::Awaiting => "awaiting",
            CrashCopyStatus::Answered => "answered",
        }
    }
}

/// `restore.status` 의 `state_file` 칸(TRD §6-5) — 이 실행의 부팅이 `state.json` 을 어떻게 읽었나. 부팅 단계 ⑥ 이
/// 정하고 그 실행 내내 바뀌지 않으며, 크래시 사본과 무관하게 늘 값이다. wire 철자는 `"ok"` · `"unreadable"` ·
/// `"corrupt_copied_aside"` · `"corrupt_not_copied"`.
///
/// - `Ok` = 읽었거나 파일이 없었다. 사본을 못 떠 이 실행이 저장하지 않는 경우(가드 ⅱ)도 이 값이다 — 이 칸은
///   `state.json` 읽기만 싣는다. 이 실행이 저장하나는 이 칸이 아니라 [`RestoreStatusView::saves`] 가 말한다.
/// - `Unreadable` = 읽기 자체가 실패했다(잠김 재시도 뒤 · 권한 — 가드 ⅰ). 파일은 그대로 두고, 이 실행은 화면 상태를
///   하나도 저장하지 않으며, 다음 부팅이 다시 판정한다.
/// - `CorruptCopiedAside` = 못 쓰는 파일(손상 · 이 판이 못 읽는 새 판 · 상한 초과 · UTF-8 아님)이라
///   `state.json.corrupt` 로 떠 두고 기본 화면으로 시작했다.
/// - `CorruptNotCopied` = 못 쓰는 파일(손상 · 이 판이 못 읽는 새 판 · 상한 초과 · UTF-8 아님)인데 떠 두지 못한 채
///   기본 화면으로 시작했다 — 원본은 백업 없이 덮인다(D8). 이미 있는 `state.json.corrupt` 는 앞선 시작이 떠 둔
///   것이지 이번 원본의 백업이 아니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StateFileStatus {
    #[default]
    Ok,
    Unreadable,
    CorruptCopiedAside,
    CorruptNotCopied,
}

impl StateFileStatus {
    /// serde 철자 그대로 — [`CrashCopyStatus::as_wire`] 와 같은 까닭.
    pub fn as_wire(self) -> &'static str {
        match self {
            StateFileStatus::Ok => "ok",
            StateFileStatus::Unreadable => "unreadable",
            StateFileStatus::CorruptCopiedAside => "corrupt_copied_aside",
            StateFileStatus::CorruptNotCopied => "corrupt_not_copied",
        }
    }
}

/// 부팅 단계 ⑥ 이 넘기는 답 없는 크래시 사본.
#[derive(Clone)]
pub struct CrashCopy {
    /// 수락이 복원할 원문 — 부팅이 읽은 사본 바이트, 가드 ⅱ(떠야 할 사본을 못 떴다)면 메모리에 쥔 `state.json`
    /// 원문이다(TRD §6-5 ⑥ · L2).
    pub text: String,
    /// `codec::crash_copy_hash(&text)`.
    pub hash: String,
    /// `text` 를 읽은 것.
    pub file: StateFile,
}

// 사본 원문(≤4 MiB)을 찍지 않는다 — 길이와 해시만.
impl fmt::Debug for CrashCopy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CrashCopy")
            .field("text_len", &self.text.len())
            .field("hash", &self.hash)
            .field("windows", &self.file.windows.len())
            .finish()
    }
}

/// `restore_status` · `restore.status` 의 답. `saved_at_ms` · `windows` · `tabs` · `durable` 은 `Awaiting` 일 때만
/// 값이고 그 밖은 `null` 이다. `saves` · `state_file` 은 크래시 사본과 무관하게 늘 값이고, 부팅 단계 ⑥ 이 정한 뒤
/// 그 실행 내내 바뀌지 않는다(답해도 · 되돌려도).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct RestoreStatusView {
    pub crash_copy: CrashCopyStatus,
    /// 사본을 적은 시각 — 유닉스 밀리초.
    // ts-rs 의 u64 기본 매핑은 bigint 이나 serde_json 은 number 로 싣는다.
    #[ts(type = "number | null")]
    pub saved_at_ms: Option<u64>,
    /// 사본의 창 수 — main + 팝아웃([`AnswerReply::restored_windows`] 와 같은 셈).
    pub windows: Option<u32>,
    /// 사본의 모든 창의 탭 수 합.
    pub tabs: Option<u32>,
    /// 묻는 동안의 [`Self::saves`] 와 같은 값이다 — `true` = 가드가 아니다: 답(수락 · 거절)을 디스크에 붙이고 사본을
    /// 지우려 한다(성공은 보장하지 않는다 — 실제로 붙었는지는 [`AnswerReply::durable`] 이 말한다). `false` = 가드다:
    /// 답해도 크래시 때 화면이 디스크에 남아(사본 또는 정상 종료 표시 없는 `state.json`) 다음 부팅이 다시 묻는다.
    pub durable: Option<bool>,
    /// 이 실행이 화면 상태를 저장하나 — 부팅 가드(TRD §6-5 ③)로 정한다. `true` = 가드가 아니다: 기록기를 띄우려
    /// 한다(띄우기 · 쓰기 성공은 보장하지 않는다 — ⑦ 에서 기록기를 못 띄운 실행도 `true` 다). `false` = 가드다: 이
    /// 실행은 화면 상태를 하나도 저장하지 않고 다음 부팅이 다시 판정한다 — 가드 ⅰ(`state_file` 이 `Unreadable`) ·
    /// 가드 ⅱ(떠야 할 크래시 사본을 못 떴다 — `state_file` 은 `Ok` 다).
    pub saves: bool,
    pub state_file: StateFileStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerConflict {
    /// `Awaiting` 이 아니다(사본 없음 · 이미 답함) — 지금 상태를 싣는다.
    NotAwaiting(CrashCopyStatus),
    /// 다른 답이 처리 중이다(TRD §6-7 「다른 답이 처리 중이면 CONFLICT」).
    InFlight,
}

/// 처리 중인 답 하나의 표지 — 복제할 수 없다. [`RestoreService::finish_answer`] 로 끝맺는다(버리면 되돌린다 —
/// 모듈 머리).
pub struct AnswerTicket {
    cell: Arc<Mutex<Cell>>,
    copy: Arc<CrashCopy>,
    generation: u64,
    settled: bool,
}

impl AnswerTicket {
    pub fn copy(&self) -> &CrashCopy {
        &self.copy
    }
}

// 사본 원문(≤4 MiB)을 찍지 않는다.
impl fmt::Debug for AnswerTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnswerTicket")
            .field("hash", &self.copy.hash)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Drop for AnswerTicket {
    fn drop(&mut self) {
        if !self.settled {
            tracing::warn!(
                module = "state",
                hash = %self.copy.hash,
                "복원 답이 끝맺지 않고 사라졌다 — 처리 중 표지를 되돌린다"
            );
            settle(
                &self.cell,
                self.generation,
                &self.copy.hash,
                AnswerEnd::RolledBack,
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerEnd {
    /// 수락이 커밋됐거나 거절했다 — 상태가 `Answered` 가 된다(알림이 난다).
    Answered,
    /// 수락이 커밋 전에 실패했다(준비 · 창 만들기) — 처리 중 표지만 풀고 `Awaiting` 그대로다. 그래도 `Awaiting` 을
    /// 알린다 — 그 사이 `InFlight` 로 거절당한 쪽은 이 알림을 받아야 다시 답할 수 있음을 안다.
    RolledBack,
}

/// 부팅 ⑥ 의 결정 · `Answered` · 되돌림(`Awaiting` 그대로) 뒤에 불린다. ★아무 락도 쥐지 않은 채 불린다★ — 구현은
/// 서비스를 다시 불러도 된다. 버려진 표지의 되돌림이면 `Drop` 안(패닉으로 풀리는 중일 수 있다)에서 불린다.
pub trait RestoreNotifier: Send + Sync {
    fn changed(&self, status: CrashCopyStatus);
}

pub struct RestoreService {
    cell: Arc<Mutex<Cell>>,
}

struct Cell {
    phase: Phase,
    /// [`RestoreService::set_boot`] 마다 오른다 — 앞 세대의 표지가 지금 상태를 바꾸지 못하게.
    generation: u64,
    notifier: Option<Arc<dyn RestoreNotifier>>,
    state_file: StateFileStatus,
    /// 묻는 동안의 `durable` 도 이 칸을 싣는다 — 둘이 갈라질 자리를 두지 않는다.
    saves: bool,
}

enum Phase {
    None,
    Awaiting {
        copy: Arc<CrashCopy>,
        in_flight: bool,
    },
    Answered,
}

impl Phase {
    fn status(&self) -> CrashCopyStatus {
        match self {
            Phase::None => CrashCopyStatus::None,
            Phase::Awaiting { .. } => CrashCopyStatus::Awaiting,
            Phase::Answered => CrashCopyStatus::Answered,
        }
    }
}

fn lock(cell: &Mutex<Cell>) -> MutexGuard<'_, Cell> {
    // 락 안에서는 칸 대입만 한다 — 패닉으로 반쯤 바뀐 상태가 없다. 표지 되돌리기가 `Drop` 에서도 돌아야 한다.
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

fn notify(notifier: Option<Arc<dyn RestoreNotifier>>, status: CrashCopyStatus) {
    if let Some(notifier) = notifier {
        notifier.changed(status);
    }
}

enum Settled {
    Answered(Option<Arc<dyn RestoreNotifier>>),
    RolledBack(Option<Arc<dyn RestoreNotifier>>),
    /// 표지가 지금 상태의 것이 아니다 — 아무것도 바꾸지 않았다.
    Stale,
}

fn settle(cell: &Mutex<Cell>, generation: u64, hash: &str, end: AnswerEnd) {
    let settled = {
        let mut cell = lock(cell);
        let in_flight = matches!(
            cell.phase,
            Phase::Awaiting {
                in_flight: true,
                ..
            }
        );
        if cell.generation != generation || !in_flight {
            Settled::Stale
        } else {
            match end {
                AnswerEnd::Answered => {
                    cell.phase = Phase::Answered;
                    Settled::Answered(cell.notifier.clone())
                }
                AnswerEnd::RolledBack => {
                    if let Phase::Awaiting { in_flight, .. } = &mut cell.phase {
                        *in_flight = false;
                    }
                    Settled::RolledBack(cell.notifier.clone())
                }
            }
        }
    };
    match settled {
        // 답의 결과 한 줄은 그 답을 실행한 쪽이 남긴다(`RestoreCoordinator::answer` — 수락 여부 · 디스크에 붙었나).
        Settled::Answered(notifier) => notify(notifier, CrashCopyStatus::Answered),
        Settled::RolledBack(notifier) => {
            tracing::info!(
                module = "state",
                hash = %hash,
                "복원 답을 되돌렸다 — 다시 답할 수 있다"
            );
            notify(notifier, CrashCopyStatus::Awaiting);
        }
        Settled::Stale => tracing::warn!(
            module = "state",
            hash = %hash,
            ?end,
            "지금 상태의 것이 아닌 복원 답 표지 — 무시한다"
        ),
    }
}

impl Default for RestoreService {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreService {
    /// 상태 `None` · `state_file` `Ok` · `saves` `true` · 알림 없음 — 부팅 단계 ⑥ 전의 값은 알릴 것이 없는 쪽이다
    /// (창은 ⑥ 뒤에 생겨 이 값을 보지 못한다 — I5).
    pub fn new() -> Self {
        Self {
            cell: Arc::new(Mutex::new(Cell {
                phase: Phase::None,
                generation: 0,
                notifier: None,
                state_file: StateFileStatus::Ok,
                saves: true,
            })),
        }
    }

    pub fn set_notifier(&self, notifier: Arc<dyn RestoreNotifier>) {
        lock(&self.cell).notifier = Some(notifier);
    }

    /// 부팅 단계 ⑥ 에서만 부른다(I5) — `copy` 가 `None` = 물을 사본이 없다 · `saves` = 이 실행이 화면 상태를 저장하나
    /// (가드가 아니다 — [`RestoreStatusView::saves`]). 처리 중이던 표지는 무효가 된다.
    pub fn set_boot(&self, copy: Option<CrashCopy>, state_file: StateFileStatus, saves: bool) {
        let hash = copy.as_ref().map(|copy| copy.hash.clone());
        let (again, status, notifier) = {
            let mut cell = lock(&self.cell);
            let again = cell.generation > 0;
            cell.generation += 1;
            cell.state_file = state_file;
            cell.saves = saves;
            cell.phase = match copy {
                Some(copy) => Phase::Awaiting {
                    copy: Arc::new(copy),
                    in_flight: false,
                },
                None => Phase::None,
            };
            (again, cell.phase.status(), cell.notifier.clone())
        };
        if again {
            tracing::warn!(
                module = "state",
                "복원 상태를 부팅 단계 밖에서 다시 정했다 — 처리 중이던 답은 무효가 된다"
            );
        }
        match hash {
            Some(hash) => tracing::info!(
                module = "state",
                hash = %hash,
                saves,
                "답하지 않은 크래시 사본이 있다 — 복원할지 묻는다"
            ),
            None => tracing::info!(module = "state", "물을 크래시 사본이 없다"),
        }
        notify(notifier, status);
    }

    pub fn status(&self) -> RestoreStatusView {
        let (status, copy, state_file, saves) = {
            let cell = lock(&self.cell);
            let copy = match &cell.phase {
                Phase::Awaiting { copy, .. } => Some(Arc::clone(copy)),
                _ => None,
            };
            (cell.phase.status(), copy, cell.state_file, cell.saves)
        };
        let Some(copy) = copy else {
            return RestoreStatusView {
                crash_copy: status,
                saved_at_ms: None,
                windows: None,
                tabs: None,
                durable: None,
                saves,
                state_file,
            };
        };
        let strips: Vec<usize> = copy
            .file
            .windows
            .iter()
            .map(|window| match &window.kind {
                WindowKind::Main(strip) | WindowKind::Popout(strip) => strip.tabs.len(),
            })
            .collect();
        RestoreStatusView {
            crash_copy: status,
            saved_at_ms: Some(copy.file.saved_at_ms),
            windows: Some(count(strips.len())),
            tabs: Some(count(strips.iter().sum())),
            durable: Some(saves),
            saves,
            state_file,
        }
    }

    /// 처리 중 표지를 세우고 사본을 건넨다.
    pub fn begin_answer(&self) -> Result<AnswerTicket, AnswerConflict> {
        let mut cell = lock(&self.cell);
        let generation = cell.generation;
        match &mut cell.phase {
            Phase::Awaiting { copy, in_flight } if !*in_flight => {
                *in_flight = true;
                Ok(AnswerTicket {
                    cell: Arc::clone(&self.cell),
                    copy: Arc::clone(copy),
                    generation,
                    settled: false,
                })
            }
            Phase::Awaiting { .. } => Err(AnswerConflict::InFlight),
            phase => Err(AnswerConflict::NotAwaiting(phase.status())),
        }
    }

    pub fn finish_answer(&self, mut ticket: AnswerTicket, end: AnswerEnd) {
        debug_assert!(
            Arc::ptr_eq(&self.cell, &ticket.cell),
            "다른 서비스가 낸 표지"
        );
        ticket.settled = true;
        settle(&ticket.cell, ticket.generation, &ticket.copy.hash, end);
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ── 복원 조율자 — 런타임 수락 · 거절(TRD §6-7 ①–⑤) ─────────────────────────────

/// `restore_answer` · `restore.answer` 의 답(TRD §6-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct AnswerReply {
    /// 사본 화면으로 다시 그린 창 수 — main + 모델에 남은 새 팝아웃(화면을 바꾸기 전에 닫힌 창은 빠진다) · 거절은 `0`.
    pub restored_windows: u32,
    /// 마감 안에 그 답을 실은 `state.json` 이 발행됐다. `false` = 마감 안에 디스크에 붙었다고 확인하지 못했다 —
    /// 다음 부팅이 다시 물을 수 있다. 기록기가 없으면(가드) 반드시 다시 묻고, 쓰기 실패 · 마감 초과면 기록기가 그
    /// 해시를 쥔 채 뒤 쓰기에서 붙일 수도 있다(`saver` 머리 「해결 칸」).
    pub durable: bool,
}

#[derive(Debug)]
pub enum AnswerError {
    /// `awaiting` 이 아니다 · 다른 답이 처리 중이다 → 버스 `CONFLICT`.
    Conflict(AnswerConflict),
    /// 셸이 아직 뜨는 중이라 창 포트가 없어 수락을 받지 못했다 — 아무것도 안 바뀌었고 `awaiting` 그대로다. 셸 setup
    /// 이 끝나면 같은 답이 선다 → 버스 `INTERNAL`. [`Self::Internal`] 과 코드는 같고 문구만 다르다(「곧 다시 답하라」).
    NotReady,
    /// 수락이 커밋 전에 실패했다(준비 · 창 만들기 · 커밋 거절) — 아무것도 안 바뀌었고 `awaiting` 그대로다 → 버스
    /// `INTERNAL`.
    Internal(String),
}

// 버스 오류의 `message` 이자 Tauri 껍데기의 오류 문자열 — 종류는 코드(버스)나 `restore_status` 로 가르고 이 글로
//   가르지 않는다.
impl fmt::Display for AnswerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnswerError::Conflict(AnswerConflict::NotAwaiting(CrashCopyStatus::Answered)) => {
                f.write_str("the crash copy was already answered in this run (crash_copy is answered)")
            }
            // `begin_answer` 는 `Awaiting` 을 싣지 않는다 — 남는 것은 `None` 이다.
            AnswerError::Conflict(AnswerConflict::NotAwaiting(_)) => {
                f.write_str("there is no crash copy to answer in this run (crash_copy is none)")
            }
            AnswerError::Conflict(AnswerConflict::InFlight) => f.write_str(
                "another answer is still being processed — read restore.status after it ends",
            ),
            AnswerError::NotReady => f.write_str(
                "the dashboard is still starting, so the screen was not restored and the crash copy still awaits an answer — answer again shortly",
            ),
            AnswerError::Internal(reason) => write!(
                f,
                "the screen was not restored and the crash copy still awaits an answer — answer again: {reason}"
            ),
        }
    }
}

/// 런타임 복원 수락의 OS 창 포트(TRD §6-7 ② ④) — 운영 = `placement::TauriRestoreWindows`, 시험 = 가짜.
///
/// ★전부 아무 락도 쥐지 않은 채 불린다★ — 창 만들기 · 세터는 이벤트 루프를 기다리고, `destroy` 의 `Destroyed`
/// 처리 · `record_placement` · `set_shown` 의 사용량 관심 알림은 레이아웃 락을 잡는다(`layout::WindowHost` 와 같은
/// 규율).
pub trait RestoreWindows: Send + Sync {
    /// 우리 앱이 지금 포커스를 가졌나(전경 창이 우리 창이다) — 못 읽으면 `false`.
    fn app_has_focus(&self) -> bool;
    /// 지금 모니터 — 못 읽으면 빈 목록(저장된 자리를 모두 버린다).
    fn monitors(&self) -> Vec<MonitorArea>;
    /// 팝아웃 창 `label` 을 숨긴 채 만들고 `at`(저장된 보통 자리 · 그것이 갈 모니터)에 놓는다 — `None` = label 의
    /// 기본 자리. `Err` = 창이 없다(세운 미룸도 거뒀다).
    ///
    /// `maximized` = 사본의 최대화를 그 창이 처음 보일 때로 미룬다 — 숨은 창을 최대화하면 보였다 숨는다. 미룬 동안
    /// 그 창의 자리는 적지 않고([`Self::record_placement`] · `Moved` · `Resized`), 보이기(`set_shown` `true` ·
    /// 트레이 보이기)가 최대화를 입히고 거둔다. 창이 소멸하면 거둔다. ★미룸은 창을 만들기 전에 세운다★ — 만든 뒤에
    /// 세우면 그 사이 트레이 「보이기」가 드러낸 창을 사용자가 닫았을 때 소멸 정리가 미룸보다 먼저 지나가 미룸이
    /// 남는다.
    fn open_hidden(
        &self,
        label: &str,
        at: Option<(WindowBounds, Landing)>,
        maximized: bool,
    ) -> Result<(), String>;
    /// 그 창이 지금 보이나 — `None` = 창이 없다. 보임 여부를 못 읽으면 `Some(true)`(창은 있다).
    fn visibility(&self, label: &str) -> Option<bool>;
    /// main 에 사본의 보통 자리(`at` — `None` = 지금 자리에 둔다)와 최대화를 입힌다. 최대화는 양쪽으로 따른다 —
    /// `false` 면 최대화를 풀고 미뤄 둔 최대화도 거둔다. 숨은 main 의 최대화는 처음 보일 때로 미루되 ★그 미룸을 자리를
    /// 입히기 전에 세운다★ — 뒤에 세우면 자리 입히기가 낸 `Moved` · `Resized` 가 그 사이 「최대화 아님」으로 적힌다.
    /// 보이는 main 도 같은 미룸을 세운 채 입히고 입혀졌을 때만 거둔다 — ★못 입혔으면 남아★ 그동안 main 의 자리를 적지
    /// 않고([`Self::record_placement`] 포함) 다음 보이기가 다시 입힌다.
    fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool);
    /// 보이거나(`true`) 숨기고 사용량 관심에 그 보임을 알린다 — 트레이 보이기 · 숨기기와 같은 차례다(ADR-0229 ·
    /// `tray::actions::for_each_ui_window`). 보일 때 미뤄 둔 최대화가 있으면 입힌다. 창이 없으면 아무것도 안 한다.
    fn set_shown(&self, label: &str, shown: bool);
    /// 그 창의 지금 자리를 모델에 적는다 — 창이 모델에 든 뒤에 부른다(그 전의 `Moved` 는 버려진다). 최대화를 미뤄
    /// 둔 동안의 창은 적지 않는다.
    fn record_placement(&self, label: &str);
    /// 그 창에 포커스를 준다 — 다른 앱이 앞에 있어도 끌어온다. 숨었거나 최소화된 창이면 아무 일도 없다(tao 0.35.3
    /// Windows `set_focus`). 창이 없으면 아무것도 안 한다.
    fn focus(&self, label: &str);
    /// 창이 없으면 `Ok`.
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// 구독 재계산 포트를 쓸 때마다(락을 잡기 전에) 찾는다 — `None` = 데몬 클라이언트가 없다: 재계산을 건너뛴다(출력이
/// 올 길이 없다). 포트를 꽂을 때 한 번만 찾으면 그 뒤에 등록된 클라이언트를 조용히 건너뛴다.
pub trait SubscriptionSource: Send + Sync {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>>;
}

/// 조율자가 셸이 선 뒤에 받는 포트 — 창 포트 · 알림 · 구독 원천 · 테마 밀기 모두 `AppHandle` 이 있어야 선다.
pub struct RestorePorts {
    pub windows: Arc<dyn RestoreWindows>,
    pub events: Arc<dyn LayoutEvents>,
    pub subs: Arc<dyn SubscriptionSource>,
    /// 셸에 하나인 그 유효 테마 — 조율자와 같은 모델을 본다.
    pub themes: ThemeControl,
}

/// 런타임 복원 수락 · 거절의 단일 경로(TRD §6-7 ①–⑤) — 사람(Tauri 껍데기)과 LLM(버스 `restore.answer`)이 같은
/// 인스턴스를 부른다.
///
/// - ★커밋 지점은 하나다(③ · `ViewManager` 락 하나 안)★ — 그 앞(① 준비 · ② 숨은 창 만들기)의 실패는 만든 창을
///   거두고 `awaiting` 으로 되돌리고, 그 뒤(④ 테마 밀기 · 알림 · 자리 · 보이기 · 사라진 팝아웃 지우기 · 옛 팝아웃
///   거두기)의 실패는 로그만 남긴다.
/// - ★복원한 팝아웃은 main 의 보임을 따른다(ADR-0229)★ — 숨은 main 곁에 팝아웃만 뜨지 않게 숨긴 채 두고, 트레이
///   「보이기」가 함께 드러낸다. 사본의 최대화는 그 창이 처음 보일 때 입힌다([`RestoreWindows::open_hidden`]).
/// - ★락 순서★: 서비스 칸 · 세션 칸은 잎이다. 창 포트 · 알림 · 기록기 기다리기는 어느 락도 쥐지 않고 한다.
/// - 포트([`RestorePorts`])는 빌더에서 만든 뒤 셸 setup 끝에 [`Self::attach`] 로 받는다 — 그 전의 수락은
///   [`AnswerError::NotReady`](`awaiting` 그대로 · 다시 답할 수 있다)이고 거절은 포트 없이 선다.
pub struct RestoreCoordinator {
    service: Arc<RestoreService>,
    layout: LayoutState,
    session: Arc<StateSession>,
    /// 런타임 팝아웃과 같은 발급기 — 새 label 이 떠 있는 창과 겹치지 않는다(§6-3).
    labels: Arc<dyn LabelSource>,
    ports: OnceLock<RestorePorts>,
}

// ① 의 결과 — 아직 아무것도 바꾸지 않았다.
struct Prepared {
    layout: ViewManager,
    main: WindowAttrs,
    // 새 팝아웃(label 발급 순) — 그 창의 사본 속성.
    popouts: Vec<(WindowLabel, WindowAttrs)>,
}

// ③ 이 락 안에서 뜬 것 — ④ 가 락 밖에서 쓴다.
struct Committed {
    removed: Vec<WindowLabel>,
    layouts: Vec<ViewSnapshot>,
    tabs: Vec<WindowTabsPayload>,
}

// ② 앞에서 읽은 OS 쪽 사정 — ② ④ 가 쓴다.
struct Desktop {
    monitors: Vec<MonitorArea>,
    app_had_focus: bool,
}

impl RestoreCoordinator {
    pub fn new(
        service: Arc<RestoreService>,
        layout: LayoutState,
        session: Arc<StateSession>,
        labels: Arc<dyn LabelSource>,
    ) -> Self {
        Self {
            service,
            layout,
            session,
            labels,
            ports: OnceLock::new(),
        }
    }

    /// 셸 setup 끝에서 한 번 — 두 번째 부름은 버린다.
    pub fn attach(&self, ports: RestorePorts) {
        if self.ports.set(ports).is_err() {
            tracing::warn!(
                module = "state",
                "복원 조율자의 포트를 두 번 꽂으려 했다 — 처음 것을 쓴다"
            );
        }
    }

    pub fn status(&self) -> RestoreStatusView {
        self.service.status()
    }

    /// 크래시 사본에 답한다 — `accept` 면 화면을 사본으로 바꾸고(①–④), 둘 다 답을 확정해 기록기에 붙인다(⑤).
    ///
    /// ★막는다 — 메인(이벤트 루프) 스레드에서 부르지 말 것★: 창을 만들고 · 놓고 · 거두는 동안 이벤트 루프를
    /// 기다리고, 기록기 답을 마감([`saver::REPLY_DEADLINE`](super::saver::REPLY_DEADLINE) — 2초)까지 기다린다.
    /// 메인에서 부르면 창 만들기가 자기 자신을 기다려 멈춘다. 부르는 쪽은 async 명령 + `spawn_blocking` 으로 넘긴다.
    /// 아무 락도 쥐지 않고 부른다.
    pub fn answer(&self, accept: bool) -> Result<AnswerReply, AnswerError> {
        let ticket = self.service.begin_answer().map_err(AnswerError::Conflict)?;
        let hash = ticket.copy().hash.clone();
        let restored_windows = if accept {
            let Some(ports) = self.ports.get() else {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "셸이 아직 뜨는 중이라 복원 수락을 받지 못했다 — 화면은 그대로이고 다시 답할 수 있다"
                );
                self.service.finish_answer(ticket, AnswerEnd::RolledBack);
                return Err(AnswerError::NotReady);
            };
            match self.accept(ports, ticket.copy()) {
                Ok(windows) => windows,
                Err(reason) => {
                    tracing::warn!(
                        module = "state",
                        hash = %hash,
                        reason = %reason,
                        "복원 수락이 커밋 전에 실패했다 — 화면은 그대로이고 다시 답할 수 있다"
                    );
                    self.service.finish_answer(ticket, AnswerEnd::RolledBack);
                    return Err(AnswerError::Internal(reason));
                }
            }
        } else {
            0
        };
        // ⑤ — 알림(`restore:changed`)은 서비스가 락 밖에서 낸다.
        self.service.finish_answer(ticket, AnswerEnd::Answered);
        let durable = self.session.resolve_crash_copy(hash.clone()) == ResolveResult::Durable;
        tracing::info!(
            module = "state",
            hash = %hash,
            accept,
            restored_windows,
            durable,
            "크래시 사본에 답했다"
        );
        Ok(AnswerReply {
            restored_windows,
            durable,
        })
    }

    // ①–④. `Err` = 커밋 전 실패 — 만든 창은 거뒀다.
    fn accept(&self, ports: &RestorePorts, copy: &CrashCopy) -> Result<u32, String> {
        let prepared = self.prepare(copy)?;
        // 포커스는 창을 하나라도 만지기 전에 읽는다 — 보인 창이 활성화를 가져간 뒤에 읽으면 답 전에 다른 앱이 앞에
        //   있었던 것을 놓친다.
        let desktop = Desktop {
            app_had_focus: ports.windows.app_has_focus(),
            monitors: ports.windows.monitors(),
        };
        let created =
            open_hidden_popouts(ports.windows.as_ref(), &prepared.popouts, &desktop.monitors)?;

        let Prepared {
            layout,
            main,
            popouts,
        } = prepared;
        let committed = match self.commit(ports, layout) {
            Ok(committed) => committed,
            Err(reason) => {
                destroy_all(ports.windows.as_ref(), &created, Leftover::HiddenRestored);
                return Err(reason);
            }
        };
        // 사본의 창 테마가 모델에 들었다 — 떠 있는 main 과, 커밋 전(모델에 들기 전)에 첫 값을 당긴 새 팝아웃이 그 값을
        //   받게 민다. 보이기(④) 앞이라 새 팝아웃이 옛 값으로 비치지 않는다(TRD S21-storage §6-7).
        // ADR-0265
        ports.themes.push();

        let kept = self.after_commit(ports, committed, main, &popouts, &desktop);
        Ok(count(1 + kept))
    }

    // ① — 부수효과 없음(발급기 번호만 쓴다 — 단조라 버려도 된다).
    fn prepare(&self, copy: &CrashCopy) -> Result<Prepared, String> {
        let Restored { layout, warnings } =
            ViewManager::from_persisted(copy.file.windows.clone(), self.labels.as_ref());
        warnings.iter().for_each(report_restore_warning);
        if let Some(RestoreWarning::Internal(reason)) = warnings
            .iter()
            .find(|w| matches!(w, RestoreWarning::Internal(_)))
        {
            return Err(format!("복원 모델을 세우지 못했다: {reason}"));
        }
        let main = layout
            .window_attrs(MAIN_WINDOW_LABEL)
            .map_err(|e| format!("복원 모델에 main 이 없다: {e}"))?;
        let mut popouts: Vec<(WindowLabel, WindowAttrs)> = layout
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(label, window)| (label.clone(), window.attrs))
            .collect();
        // 같은 접두 + 십진 순번이라 (길이, 글자) 순이 발급 순이다.
        popouts.sort_by_key(|(label, _)| (label.len(), label.clone()));
        {
            let live = self
                .layout
                .0
                .lock()
                .map_err(|_| "레이아웃 락에 독이 들었다".to_string())?;
            if let Some((label, _)) = popouts.iter().find(|(l, _)| live.windows.contains_key(l)) {
                return Err(format!(
                    "새 팝아웃 label {label} 이 떠 있는 창과 겹친다 — label 발급기가 런타임 팝아웃과 다르다"
                ));
            }
        }
        Ok(Prepared {
            layout,
            main,
            popouts,
        })
    }

    // ③ — 유일한 커밋 지점. 모델 교체 · 알림 재료 · 구독 재계산을 한 임계구역에서(`layout::apply` 「락 규율」).
    fn commit(&self, ports: &RestorePorts, restored: ViewManager) -> Result<Committed, String> {
        let subs = ports.subs.current();
        let mut mgr = self.layout.0.lock().map_err(|_| {
            tracing::error!(
                module = "state",
                "레이아웃 락에 독이 들어 복원한 화면을 들이지 못했다"
            );
            "레이아웃 락에 독이 들었다".to_string()
        })?;
        let removed = mgr
            .adopt_restored(restored)
            .map_err(|e| format!("복원 모델을 들이지 못했다: {e}"))?;
        let mut labels = mgr.list_windows();
        labels.sort_by_key(|label| {
            (
                label.as_str() != MAIN_WINDOW_LABEL,
                label.len(),
                label.clone(),
            )
        });
        let mut layouts = Vec::new();
        let mut tabs = Vec::new();
        for label in &labels {
            if let Ok(snapshot) = mgr.list_tabs(label) {
                layouts.extend(
                    snapshot
                        .tabs
                        .iter()
                        .filter_map(|meta| mgr.snapshot(meta.id).ok()),
                );
                tabs.push(WindowTabsPayload::from(snapshot));
            }
        }
        if let Some(subs) = &subs {
            subs.resync(&mgr);
        }
        Ok(Committed {
            removed,
            layouts,
            tabs,
        })
    }

    // ④ — 되돌리지 않는다. 실패는 포트 구현이 로그로 남긴다. 돌려주는 것 = 모델에 남은 새 팝아웃 수.
    fn after_commit(
        &self,
        ports: &RestorePorts,
        committed: Committed,
        main: WindowAttrs,
        popouts: &[(WindowLabel, WindowAttrs)],
        desktop: &Desktop,
    ) -> usize {
        let Committed {
            removed,
            layouts,
            tabs,
        } = committed;
        let monitors = desktop.monitors.as_slice();
        for snapshot in &layouts {
            ports.events.layout_updated(snapshot);
        }
        for payload in &tabs {
            ports.events.window_tabs_updated(payload);
        }

        let windows = ports.windows.as_ref();
        let main_at = main.bounds.and_then(|bounds| {
            land(MAIN_WINDOW_LABEL, bounds, monitors, Fallback::StayPut).map(|at| (bounds, at))
        });
        windows.place_main(main_at, main.maximized);
        // 사본의 자리를 버렸거나 못 입혔으면 모델이 창이 간 적 없는 자리를 쥔다 — 지금 자리로 맞춘다. main 의 최대화를
        //   못 입혔으면 main 은 적히지 않는다(미룸이 남는다 — `place_main`).
        windows.record_placement(MAIN_WINDOW_LABEL);

        let main_shown = windows.visibility(MAIN_WINDOW_LABEL) != Some(false);
        let mut vanished = Vec::new();
        for (label, attrs) in popouts {
            // ② 와 ③ 사이에 트레이 「보이기」가 드러낸 창을 사용자가 닫았으면 그 `Destroyed` 정리는 모델에 아직 없던
            //   label 이라 지나갔다 — 여기서 안 지우면 OS 창 없는 창이 모델에 남는다.
            if windows.visibility(label).is_none() {
                vanished.push(label.clone());
                continue;
            }
            windows.set_shown(label, main_shown);
            // 숨긴 채 두는 최대화 팝아웃은 적지 않는다 — 그 최대화는 ② 가 보일 때로 미뤘고, 지금 자리(보통)를 적으면
            //   모델에서 사본의 최대화가 지워진다. 보였으면 `set_shown` 이 최대화를 입혔다.
            if main_shown || !attrs.maximized {
                windows.record_placement(label);
            }
        }
        self.forget_vanished(ports, &vanished);
        // 그 창의 `Destroyed` 정리는 이미 모델에 없는 label 이라 재계산만 한다(`popout::drop_window_in_model`).
        destroy_all(windows, &removed, Leftover::Replaced);
        let kept = popouts.len() - vanished.len();
        // 보인 팝아웃마다 포커스를 가져갔다 — 「복원」을 누른 사람 앞에 main 을 되돌린다(트레이 보이기가 main 을
        //   마지막에 두는 것과 같은 까닭 — ADR-0229). 보인 창이 없으면 포커스가 옮지 않았으니 끌어오지 않는다.
        // ★답 전에 우리 창이 포커스를 가졌고 지금도 가졌을 때만★ — 다른 앱이 앞에 있으면 포커스 주기가 그 앱에 가짜
        //   Alt 키를 쏴 포커스를 뺏는다(tao 0.35.3 Windows `set_focus` → `force_window_active`). LLM 이 뒤에서 답하는
        //   동안 사람은 다른 앱에서 일하고 있을 수 있다. 답 전의 표본만으로는 ②–④ 사이에 사람이 다른 앱으로 옮긴 것을
        //   놓친다 — 부르기 바로 앞에서 다시 읽는다(앞의 표본은 그대로 쓴다 — 보인 팝아웃이 활성화를 가져가 이 값만으로는
        //   답 전에 다른 앱이 앞에 있었던 것을 놓친다).
        if desktop.app_had_focus && main_shown && kept > 0 && windows.app_has_focus() {
            windows.focus(MAIN_WINDOW_LABEL);
        }
        kept
    }

    // OS 창이 사라진 새 팝아웃을 모델에서 지운다 — 창 소멸 정리와 같은 일(창 지우기 + 구독 재계산을 한 락 안 —
    //   `popout::drop_window_in_model`). 그 경로처럼 프론트에는 알리지 않는다.
    fn forget_vanished(&self, ports: &RestorePorts, labels: &[WindowLabel]) {
        if labels.is_empty() {
            return;
        }
        tracing::warn!(
            module = "state",
            ?labels,
            "복원한 팝아웃 창이 화면을 바꾸기 전에 닫혀 모델에서 지운다"
        );
        let subs = ports.subs.current();
        let Ok(mut mgr) = self.layout.0.lock() else {
            tracing::error!(
                module = "state",
                ?labels,
                "레이아웃 락에 독이 들어 닫힌 복원 팝아웃을 모델에서 지우지 못했다 — 창 없는 창이 모델에 남는다"
            );
            return;
        };
        for label in labels {
            if let Err(e) = mgr.close_window(label) {
                tracing::debug!(module = "state", label = %label, error = %e, "이미 모델에 없는 복원 팝아웃");
            }
        }
        if let Some(subs) = &subs {
            subs.resync(&mgr);
        }
    }
}

// ② — 하나라도 못 만들면 만든 것을 전부 거두고 `Err`.
fn open_hidden_popouts(
    windows: &dyn RestoreWindows,
    popouts: &[(WindowLabel, WindowAttrs)],
    monitors: &[MonitorArea],
) -> Result<Vec<WindowLabel>, String> {
    let mut created = Vec::new();
    for (label, attrs) in popouts {
        let at = attrs.bounds.and_then(|bounds| {
            land(label, bounds, monitors, Fallback::DefaultPlace).map(|at| (bounds, at))
        });
        if let Err(e) = windows.open_hidden(label, at, attrs.maximized) {
            destroy_all(windows, &created, Leftover::HiddenRestored);
            return Err(format!("복원할 팝아웃 창 {label} 을 못 만들었다: {e}"));
        }
        created.push(label.clone());
    }
    Ok(created)
}

// 거두지 못한 창이 무엇인가 — 마지막 실패 로그의 문구만 가른다.
#[derive(Clone, Copy)]
enum Leftover {
    /// 되돌리기에서 거두는, 숨긴 채 만든 복원 팝아웃.
    HiddenRestored,
    /// 사본 화면으로 갈아끼우며 모델에서 지운 옛 팝아웃.
    Replaced,
}

// 하나씩 거두고, 실패하면 한 번 더 — 그래도 실패면 그 label 을 error 로 남긴다.
fn destroy_all(windows: &dyn RestoreWindows, labels: &[WindowLabel], leftover: Leftover) {
    for label in labels {
        let Err(first) = windows.destroy(label) else {
            continue;
        };
        tracing::warn!(module = "state", label = %label, error = %first, "창을 거두지 못해 한 번 더 거둔다");
        let Err(e) = windows.destroy(label) else {
            continue;
        };
        match leftover {
            Leftover::HiddenRestored => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "복원하려고 숨긴 채 만든 창을 거두지 못했다 — 모델에 없는 숨은 창이 남고, 트레이 「보이기」가 그 창을 드러낸다"
            ),
            Leftover::Replaced => tracing::error!(
                module = "state",
                label = %label,
                error = %e,
                "사본 화면으로 바꾼 뒤 옛 팝아웃 창을 거두지 못했다 — 모델에 없는 창이 화면에 남는다"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::schema::{
        PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry, STATE_VERSION,
    };
    use uuid::Uuid;

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<CrashCopyStatus>>,
        /// 알림 순간 서비스 락이 비어 있었나 — 서비스 칸을 같이 쥔다.
        probe: Mutex<Option<Arc<Mutex<Cell>>>>,
        lock_was_free: Mutex<Vec<bool>>,
    }

    impl RestoreNotifier for Recorder {
        fn changed(&self, status: CrashCopyStatus) {
            if let Some(cell) = self.probe.lock().unwrap().as_ref() {
                self.lock_was_free
                    .lock()
                    .unwrap()
                    .push(cell.try_lock().is_ok());
            }
            self.seen.lock().unwrap().push(status);
        }
    }

    impl Recorder {
        fn seen(&self) -> Vec<CrashCopyStatus> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn tab() -> TabEntry {
        TabEntry {
            id: Uuid::new_v4(),
            name: "탭".into(),
            focused_slot_id: None,
            layout: PersistedNode::Slot {
                id: Uuid::new_v4(),
                content: PersistedContent::Known(crate::layout::SlotContent::Empty),
            },
        }
    }

    fn window(id: &str, kind: WindowKind) -> WindowEntry {
        WindowEntry {
            id: id.into(),
            kind,
            theme: None,
            bounds: None,
            maximized: false,
        }
    }

    fn strip(tabs: usize) -> TabStrip {
        let tabs: Vec<TabEntry> = (0..tabs).map(|_| tab()).collect();
        TabStrip {
            active_tab: tabs[0].id,
            tabs,
        }
    }

    fn crash_copy(hash: &str) -> CrashCopy {
        CrashCopy {
            text: "원문".into(),
            hash: hash.into(),
            file: StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1234,
                clean_exit: false,
                resolved_crash_copy: None,
                windows: vec![
                    window("main", WindowKind::Main(strip(2))),
                    window("p-1", WindowKind::Popout(strip(1))),
                ],
            },
        }
    }

    fn service() -> (RestoreService, Arc<Recorder>) {
        let service = RestoreService::new();
        let recorder = Arc::new(Recorder::default());
        *recorder.probe.lock().unwrap() = Some(Arc::clone(&service.cell));
        service.set_notifier(recorder.clone());
        (service, recorder)
    }

    fn idle(status: CrashCopyStatus) -> RestoreStatusView {
        RestoreStatusView {
            crash_copy: status,
            saved_at_ms: None,
            windows: None,
            tabs: None,
            durable: None,
            saves: true,
            state_file: StateFileStatus::Ok,
        }
    }

    // ── 상태 셋 ──

    #[test]
    fn a_new_service_has_no_copy_and_refuses_an_answer() {
        let service = RestoreService::new();
        assert_eq!(service.status(), idle(CrashCopyStatus::None));
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::NotAwaiting(CrashCopyStatus::None)
        );
    }

    #[test]
    fn the_boot_without_a_copy_settles_none_and_notifies() {
        let (service, recorder) = service();
        service.set_boot(None, StateFileStatus::Ok, true);
        assert_eq!(service.status(), idle(CrashCopyStatus::None));
        assert_eq!(recorder.seen(), [CrashCopyStatus::None]);
    }

    #[test]
    fn the_boot_with_a_copy_awaits_and_the_view_counts_the_copy() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);
        assert_eq!(
            service.status(),
            RestoreStatusView {
                crash_copy: CrashCopyStatus::Awaiting,
                saved_at_ms: Some(1234),
                windows: Some(2),
                tabs: Some(3),
                durable: Some(true),
                saves: true,
                state_file: StateFileStatus::Ok,
            }
        );
        assert_eq!(recorder.seen(), [CrashCopyStatus::Awaiting]);
    }

    #[test]
    fn durable_is_saves_while_awaiting_and_null_outside_awaiting() {
        for saves in [true, false] {
            let (service, _recorder) = service();
            assert_eq!(service.status().durable, None, "none — {saves}");

            service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, saves);
            assert_eq!(service.status().durable, Some(saves), "awaiting — {saves}");

            let ticket = service.begin_answer().unwrap();
            assert_eq!(
                service.status().durable,
                Some(saves),
                "처리 중에도 awaiting — {saves}"
            );
            service.finish_answer(ticket, AnswerEnd::Answered);
            assert_eq!(service.status().durable, None, "answered — {saves}");
        }
    }

    #[test]
    fn saves_is_on_every_view_of_the_run_and_survives_the_answer() {
        for saves in [true, false] {
            let (none, _recorder) = service();
            none.set_boot(None, StateFileStatus::Ok, saves);
            assert_eq!(
                none.status(),
                RestoreStatusView {
                    saves,
                    ..idle(CrashCopyStatus::None)
                },
                "물을 사본이 없어도 선다 — {saves}"
            );

            let (service, _recorder) = service();
            service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, saves);
            let awaiting = service.status();
            assert_eq!(awaiting.saves, saves, "awaiting — {saves}");
            assert_eq!(
                awaiting.durable,
                Some(awaiting.saves),
                "묻는 동안 둘은 같다 — {saves}"
            );

            let ticket = service.begin_answer().unwrap();
            assert_eq!(service.status().saves, saves, "처리 중 — {saves}");
            service.finish_answer(ticket, AnswerEnd::RolledBack);
            assert_eq!(service.status().saves, saves, "되돌린 뒤 — {saves}");

            let ticket = service.begin_answer().unwrap();
            service.finish_answer(ticket, AnswerEnd::Answered);
            assert_eq!(
                service.status(),
                RestoreStatusView {
                    saves,
                    ..idle(CrashCopyStatus::Answered)
                },
                "답한 뒤에도 그 실행의 값 그대로 — {saves}"
            );
        }
    }

    #[test]
    fn the_boot_sets_the_state_file_status_for_every_view_of_the_run() {
        let (unread, recorder) = service();
        unread.set_boot(None, StateFileStatus::Unreadable, false);
        assert_eq!(
            unread.status(),
            RestoreStatusView {
                saves: false,
                state_file: StateFileStatus::Unreadable,
                ..idle(CrashCopyStatus::None)
            },
            "사본이 없어도 선다"
        );
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::None],
            "알림의 실을 것은 그대로"
        );

        // 못 쓸 state.json × 답 없는 사본 — 떠 두고 묻는다(§6-5 표).
        let (service, _corrupt_recorder) = service();
        service.set_boot(
            Some(crash_copy("h")),
            StateFileStatus::CorruptCopiedAside,
            true,
        );
        assert_eq!(
            service.status().state_file,
            StateFileStatus::CorruptCopiedAside
        );
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::Answered);
        assert_eq!(
            service.status(),
            RestoreStatusView {
                state_file: StateFileStatus::CorruptCopiedAside,
                ..idle(CrashCopyStatus::Answered)
            },
            "답해도 그 실행의 값 그대로"
        );
    }

    #[test]
    fn an_answer_moves_to_answered_notifies_and_refuses_a_second_answer() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let ticket = service.begin_answer().expect("답할 수 있다");
        assert_eq!(ticket.copy().hash, "h");
        assert_eq!(ticket.copy().text, "원문");
        service.finish_answer(ticket, AnswerEnd::Answered);

        assert_eq!(service.status(), idle(CrashCopyStatus::Answered));
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::NotAwaiting(CrashCopyStatus::Answered)
        );
    }

    #[test]
    fn a_second_answer_while_one_is_in_flight_conflicts() {
        let (service, _recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let _ticket = service.begin_answer().unwrap();
        assert_eq!(
            service.begin_answer().unwrap_err(),
            AnswerConflict::InFlight
        );
        assert_eq!(
            service.status().crash_copy,
            CrashCopyStatus::Awaiting,
            "처리 중에도 상태는 awaiting"
        );
    }

    #[test]
    fn a_rolled_back_answer_stays_awaiting_notifies_it_and_can_be_answered_again() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::RolledBack);

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "되돌림도 알린다 — 그 사이 InFlight 로 거절당한 쪽이 다시 답할 수 있음을 안다"
        );
        assert!(service.begin_answer().is_ok());
    }

    #[test]
    fn dropping_a_ticket_rolls_back() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        drop(service.begin_answer().unwrap());

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting]
        );
        assert!(service.begin_answer().is_ok(), "표지가 풀렸다");
    }

    #[test]
    fn a_ticket_unwound_by_a_panic_rolls_back() {
        let (service, _recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ticket = service.begin_answer().unwrap();
            panic!("수락 도중 패닉");
        }));

        assert!(result.is_err());
        assert!(service.begin_answer().is_ok());
    }

    #[test]
    fn a_ticket_from_before_a_second_boot_setting_changes_nothing() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("옛")), StateFileStatus::Ok, true);
        let stale = service.begin_answer().unwrap();

        service.set_boot(Some(crash_copy("새")), StateFileStatus::Ok, true);
        service.finish_answer(stale, AnswerEnd::Answered);

        assert_eq!(service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            recorder.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "옛 표지의 답은 알리지 않는다"
        );
        let fresh = service.begin_answer().expect("새 세대는 처리 중이 아니다");
        assert_eq!(fresh.copy().hash, "새");
    }

    #[test]
    fn the_notifier_is_called_with_no_lock_held() {
        let (service, recorder) = service();
        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::RolledBack);
        drop(service.begin_answer().unwrap());
        let ticket = service.begin_answer().unwrap();
        service.finish_answer(ticket, AnswerEnd::Answered);

        assert_eq!(
            *recorder.lock_was_free.lock().unwrap(),
            [true, true, true, true],
            "부팅 · 되돌림 · 버린 표지 · 답"
        );
    }

    #[test]
    fn the_notifier_may_call_back_into_the_service() {
        struct Reentrant(
            Mutex<Option<Arc<RestoreService>>>,
            Mutex<Vec<RestoreStatusView>>,
        );
        impl RestoreNotifier for Reentrant {
            fn changed(&self, _status: CrashCopyStatus) {
                if let Some(service) = self.0.lock().unwrap().as_ref() {
                    self.1.lock().unwrap().push(service.status());
                }
            }
        }
        let service = Arc::new(RestoreService::new());
        let notifier = Arc::new(Reentrant(
            Mutex::new(Some(service.clone())),
            Mutex::default(),
        ));
        service.set_notifier(notifier.clone());

        service.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, true);

        assert_eq!(
            notifier.1.lock().unwrap()[0].crash_copy,
            CrashCopyStatus::Awaiting,
            "알림 안에서 본 상태가 이미 정해진 값이다"
        );
        // 고리를 끊는다(서비스 ↔ 알림).
        notifier.0.lock().unwrap().take();
    }

    #[test]
    fn the_status_wire_spelling_is_snake_case() {
        let spell = |status| serde_json::to_string(&status).unwrap();
        assert_eq!(spell(CrashCopyStatus::None), "\"none\"");
        assert_eq!(spell(CrashCopyStatus::Awaiting), "\"awaiting\"");
        assert_eq!(spell(CrashCopyStatus::Answered), "\"answered\"");
        for status in [
            CrashCopyStatus::None,
            CrashCopyStatus::Awaiting,
            CrashCopyStatus::Answered,
        ] {
            assert_eq!(
                format!("\"{}\"", status.as_wire()),
                spell(status),
                "버스 철자(as_wire)와 Tauri 철자(serde)가 같다"
            );
        }
    }

    #[test]
    fn the_state_file_wire_spelling_is_snake_case() {
        let spell = |status| serde_json::to_string(&status).unwrap();
        assert_eq!(spell(StateFileStatus::Ok), "\"ok\"");
        assert_eq!(spell(StateFileStatus::Unreadable), "\"unreadable\"");
        assert_eq!(
            spell(StateFileStatus::CorruptCopiedAside),
            "\"corrupt_copied_aside\""
        );
        assert_eq!(
            spell(StateFileStatus::CorruptNotCopied),
            "\"corrupt_not_copied\""
        );
        for status in [
            StateFileStatus::Ok,
            StateFileStatus::Unreadable,
            StateFileStatus::CorruptCopiedAside,
            StateFileStatus::CorruptNotCopied,
        ] {
            assert_eq!(
                format!("\"{}\"", status.as_wire()),
                spell(status),
                "버스 철자(as_wire)와 Tauri 철자(serde)가 같다"
            );
        }
    }

    #[test]
    fn the_status_and_the_reply_serialize_as_the_wire_shapes() {
        let (service, _recorder) = service();
        assert_eq!(
            serde_json::to_value(service.status()).unwrap(),
            serde_json::json!({ "crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": true, "state_file": "ok" })
        );
        service.set_boot(
            Some(crash_copy("h")),
            StateFileStatus::CorruptNotCopied,
            true,
        );
        assert_eq!(
            serde_json::to_value(service.status()).unwrap(),
            serde_json::json!({ "crash_copy": "awaiting", "saved_at_ms": 1234, "windows": 2, "tabs": 3, "durable": true, "saves": true, "state_file": "corrupt_not_copied" })
        );

        // 가드 ⅱ 의 답한 뒤 — 이 실행이 저장하지 않는다는 사실이 `saves` 하나로 남는다.
        let guarded = RestoreService::new();
        guarded.set_boot(Some(crash_copy("h")), StateFileStatus::Ok, false);
        let ticket = guarded.begin_answer().unwrap();
        guarded.finish_answer(ticket, AnswerEnd::Answered);
        assert_eq!(
            serde_json::to_value(guarded.status()).unwrap(),
            serde_json::json!({ "crash_copy": "answered", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": false, "state_file": "ok" })
        );
        assert_eq!(
            serde_json::to_value(AnswerReply {
                restored_windows: 2,
                durable: false
            })
            .unwrap(),
            serde_json::json!({ "restored_windows": 2, "durable": false })
        );
    }

    // ── 복원 조율자 ──

    use crate::commands::popout::PopupCounter;
    use crate::layout::{tree, WindowPlacement};
    use crate::settings::SettingsService;
    use crate::state::convert::to_persisted;
    use crate::theme::{EffectiveThemes, ThemeWindows, UiSettingsPayload, UiTheme};

    const PRIMARY: MonitorArea = MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    fn rect(x: f64, y: f64, w: f64, h: f64) -> WindowBounds {
        WindowBounds::new(x, y, w, h).unwrap()
    }

    fn seen(maximized: bool, bounds: WindowBounds) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized,
            bounds: Some(bounds),
        }
    }

    fn foreign() -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({ "type": "future_slot", "from": "a newer build" })
            .as_object()
            .unwrap()
            .clone()
    }

    fn first_slot(mgr: &ViewManager, view: Uuid) -> Uuid {
        tree::first_slot_id(&mgr.views[&view].layout)
    }

    /// 앞 실행의 화면 — main 탭 「지난 탭」(모니터 위 자리 · `main_maximized`) · 팝아웃 `popouts` 개(첫째 = 모르는
    /// 내용 슬롯 · 어느 모니터에도 안 걸치는 자리 · 최대화).
    fn previous_screen(popouts: usize, main_maximized: bool) -> Vec<WindowEntry> {
        let mut prev = ViewManager::new();
        let main_view = prev.windows[MAIN_WINDOW_LABEL].active;
        prev.rename_tab(main_view, "지난 탭".into()).unwrap();
        prev.observe_window_placement(
            MAIN_WINDOW_LABEL,
            seen(false, rect(100.0, 80.0, 1200.0, 800.0)),
        )
        .unwrap();
        if main_maximized {
            prev.observe_window_placement(
                MAIN_WINDOW_LABEL,
                seen(true, rect(0.0, 0.0, 1920.0, 1040.0)),
            )
            .unwrap();
        }
        for n in 0..popouts {
            let label = format!("slot-popup-{}", 70 + n);
            let view = prev.create_window(&label).unwrap();
            if n == 0 {
                let slot = first_slot(&prev, view);
                prev.set_unknown_content(view, slot, foreign()).unwrap();
                prev.observe_window_placement(
                    &label,
                    seen(false, rect(-9000.0, 100.0, 700.0, 500.0)),
                )
                .unwrap();
                prev.observe_window_placement(&label, seen(true, rect(0.0, 0.0, 1920.0, 1040.0)))
                    .unwrap();
            }
        }
        to_persisted(&prev)
    }

    fn copy_of(windows: Vec<WindowEntry>) -> CrashCopy {
        CrashCopy {
            text: "원문".into(),
            hash: "h".into(),
            file: StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1,
                clean_exit: false,
                resolved_crash_copy: None,
                windows,
            },
        }
    }

    /// 가짜 창 포트 — 부른 순서를 적고, 부를 때마다 레이아웃 락이 비었는지 · 그 label 이 모델에 있었는지 본다.
    #[derive(Default)]
    struct FakeWindows {
        layout: LayoutState,
        calls: Mutex<Vec<String>>,
        lock_was_free: Mutex<Vec<bool>>,
        /// 이 label 의 창 만들기가 실패한다.
        fail_open: Mutex<Option<String>>,
        /// 이 label 의 창을 만들 때 같은 label 의 창이 모델에 끼어든다(③ 의 커밋 거절을 부른다).
        squat_on_open: Mutex<Option<String>>,
        /// 보임 여부가 「숨음」인 창.
        hidden: Mutex<Vec<String>>,
        /// OS 창이 없다고 답하는 창.
        gone: Mutex<Vec<String>>,
        /// 이 label 의 거두기가 늘 실패한다.
        fail_destroy: Mutex<Option<String>>,
        /// 최대화를 보일 때로 미룬 창 — 보이기가 입히고 거둔다(운영 포트의 계약).
        deferred: Mutex<Vec<String>>,
        /// 다른 앱이 앞에 있다(우리 창 어느 것도 포커스가 없다). 보인 창이 활성화를 가져간다고 쳐 보이기가 이 값을
        /// 내린다 — 포커스를 ② 뒤에 읽으면 이 값에 속는다.
        background: Mutex<bool>,
        /// 포커스 읽기의 답을 앞에서부터 하나씩 정한다 — 다 쓰면 `background` 로 답한다.
        focus_samples: Mutex<Vec<bool>>,
    }

    impl FakeWindows {
        fn new(layout: &LayoutState) -> Self {
            Self {
                layout: layout.clone(),
                ..Self::default()
            }
        }

        fn note(&self, call: String) {
            self.lock_was_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.calls.lock().unwrap().push(call);
        }

        fn in_model(&self, label: &str) -> bool {
            self.layout.0.lock().unwrap().windows.contains_key(label)
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        /// 미룬 최대화를 뺀 부름 — 팝아웃이 둘 이상이면 어느 새 label 이 사본의 최대화 팝아웃을 받는지가 매번 다르다
        /// (사본의 팝아웃 차례 = 영속 id 순).
        fn calls_but_deferrals(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| !call.starts_with("defer_maximize "))
                .collect()
        }

        fn deferred(&self) -> Vec<String> {
            self.deferred.lock().unwrap().clone()
        }

        fn listed(list: &Mutex<Vec<String>>, label: &str) -> bool {
            list.lock().unwrap().iter().any(|l| l == label)
        }
    }

    impl RestoreWindows for FakeWindows {
        fn app_has_focus(&self) -> bool {
            self.note("focused?".into());
            let mut samples = self.focus_samples.lock().unwrap();
            if samples.is_empty() {
                !*self.background.lock().unwrap()
            } else {
                samples.remove(0)
            }
        }

        fn monitors(&self) -> Vec<MonitorArea> {
            self.note("monitors".into());
            vec![PRIMARY]
        }

        // 미룸은 창을 만들기 전에 세우고, 만들기가 실패하면 거둔다(운영 포트의 계약).
        fn open_hidden(
            &self,
            label: &str,
            at: Option<(WindowBounds, Landing)>,
            maximized: bool,
        ) -> Result<(), String> {
            if maximized {
                self.note(format!(
                    "defer_maximize {label} in_model={}",
                    self.in_model(label)
                ));
                self.deferred.lock().unwrap().push(label.into());
            }
            self.note(format!(
                "open_hidden {label} at={} in_model={}",
                at.is_some(),
                self.in_model(label)
            ));
            if self.fail_open.lock().unwrap().as_deref() == Some(label) {
                self.deferred.lock().unwrap().retain(|l| l != label);
                return Err("창 생성 실패(시험)".into());
            }
            if self.squat_on_open.lock().unwrap().as_deref() == Some(label) {
                self.layout.0.lock().unwrap().create_window(label).unwrap();
            }
            Ok(())
        }

        fn visibility(&self, label: &str) -> Option<bool> {
            self.note(format!("visible? {label}"));
            if Self::listed(&self.gone, label) {
                return None;
            }
            Some(!Self::listed(&self.hidden, label))
        }

        fn place_main(&self, at: Option<(WindowBounds, Landing)>, maximized: bool) {
            self.note(format!(
                "place_main at={} maximized={maximized}",
                at.is_some()
            ));
        }

        fn set_shown(&self, label: &str, shown: bool) {
            let verb = if shown { "show" } else { "hide" };
            self.note(format!("{verb} {label} in_model={}", self.in_model(label)));
            if shown {
                *self.background.lock().unwrap() = false;
            }
            let applied = {
                let mut deferred = self.deferred.lock().unwrap();
                let pending = shown && deferred.iter().any(|l| l == label);
                deferred.retain(|l| !(pending && l == label));
                pending
            };
            if applied {
                self.note(format!("maximize {label} (deferred)"));
            }
        }

        fn record_placement(&self, label: &str) {
            self.note(format!("record {label}"));
        }

        fn focus(&self, label: &str) {
            self.note(format!("focus {label}"));
        }

        fn destroy(&self, label: &str) -> Result<(), String> {
            self.note(format!("destroy {label} in_model={}", self.in_model(label)));
            if self.fail_destroy.lock().unwrap().as_deref() == Some(label) {
                return Err("창 거두기 실패(시험)".into());
            }
            Ok(())
        }
    }

    /// 알림 — 첫 알림 순간에 레이아웃 락이 비었는지 적는다.
    struct FakeEvents {
        layout: LayoutState,
        layouts: Mutex<Vec<Uuid>>,
        tabs: Mutex<Vec<(String, u64)>>,
        first_seen: Mutex<Option<bool>>,
    }

    impl FakeEvents {
        fn note(&self) {
            let mut first = self.first_seen.lock().unwrap();
            if first.is_none() {
                *first = Some(self.layout.0.try_lock().is_ok());
            }
        }
    }

    impl LayoutEvents for FakeEvents {
        fn layout_updated(&self, snapshot: &ViewSnapshot) {
            self.note();
            self.layouts.lock().unwrap().push(snapshot.view_id);
        }

        fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
            self.note();
            self.tabs
                .lock()
                .unwrap()
                .push((tabs.label.clone(), tabs.version));
        }
    }

    /// 구독 재계산 — 레이아웃 락 안에서 불린다. 그 순간의 모델 번호를 적는다.
    #[derive(Default)]
    struct FakeSubs {
        seen: Mutex<Vec<u64>>,
    }

    impl SubscriptionSync for FakeSubs {
        fn resync(&self, mgr: &ViewManager) {
            self.seen.lock().unwrap().push(mgr.version);
        }
    }

    /// 테마 밀기의 창 쪽 — 살아 있는 웹뷰 = 모델의 창. 보낸 것과 보낼 때 레이아웃 락이 비었는지 적는다.
    struct ThemeScreen {
        layout: LayoutState,
        sent: Mutex<Vec<(String, String)>>,
        lock_was_free: Mutex<Vec<bool>>,
    }

    impl ThemeWindows for ThemeScreen {
        fn labels(&self) -> Vec<String> {
            let mut labels = self.layout.0.lock().unwrap().list_windows();
            labels.sort();
            labels
        }

        fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
            self.lock_was_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.sent
                .lock()
                .unwrap()
                .push((label.to_string(), payload.theme));
            Ok(())
        }
    }

    /// 구독 원천 — 비어 있으면 데몬 클라이언트가 아직 없는 셸이다. 찾을 때 락이 비었는지 적는다.
    struct SubsSlot {
        layout: LayoutState,
        subs: Mutex<Option<Arc<dyn SubscriptionSync>>>,
        lookups_with_lock_free: Mutex<Vec<bool>>,
    }

    impl SubscriptionSource for SubsSlot {
        fn current(&self) -> Option<Arc<dyn SubscriptionSync>> {
            self.lookups_with_lock_free
                .lock()
                .unwrap()
                .push(self.layout.0.try_lock().is_ok());
            self.subs.lock().unwrap().clone()
        }
    }

    struct Rig {
        coordinator: RestoreCoordinator,
        service: Arc<RestoreService>,
        notices: Arc<Recorder>,
        layout: LayoutState,
        labels: Arc<PopupCounter>,
        windows: Arc<FakeWindows>,
        events: Arc<FakeEvents>,
        subs: Arc<FakeSubs>,
        slot: Arc<SubsSlot>,
        theme_screen: Arc<ThemeScreen>,
        themes: ThemeControl,
    }

    impl Rig {
        /// `labels` = 조율자의 발급기 — `None` 이면 런타임 팝아웃과 같은 것(`self.labels`).
        fn new(labels: Option<Arc<PopupCounter>>) -> Self {
            let service = Arc::new(RestoreService::new());
            let notices = Arc::new(Recorder::default());
            service.set_notifier(notices.clone());
            let layout = LayoutState::new();
            let shared = Arc::new(PopupCounter::default());
            let coordinator = RestoreCoordinator::new(
                service.clone(),
                layout.clone(),
                Arc::new(StateSession::default()),
                labels.unwrap_or_else(|| shared.clone()),
            );
            let windows = Arc::new(FakeWindows::new(&layout));
            let events = Arc::new(FakeEvents {
                layout: layout.clone(),
                layouts: Mutex::default(),
                tabs: Mutex::default(),
                first_seen: Mutex::default(),
            });
            let subs = Arc::new(FakeSubs::default());
            let slot = Arc::new(SubsSlot {
                layout: layout.clone(),
                subs: Mutex::default(),
                lookups_with_lock_free: Mutex::default(),
            });
            let theme_screen = Arc::new(ThemeScreen {
                layout: layout.clone(),
                sent: Mutex::default(),
                lock_was_free: Mutex::default(),
            });
            // 쓰기를 안 연 설정 — 전역 테마는 기본값(dark)이고, 적재는 없는 폴더를 만들지 않는다.
            let settings = Arc::new(SettingsService::load_from_dir(
                &std::env::temp_dir().join("engram-restore-no-settings"),
            ));
            let themes = ThemeControl::new(
                Arc::new(EffectiveThemes::new(settings, layout.clone())),
                theme_screen.clone(),
            );
            Rig {
                coordinator,
                service,
                notices,
                layout,
                labels: shared,
                windows,
                events,
                subs,
                slot,
                theme_screen,
                themes,
            }
        }

        /// 데몬 클라이언트가 선 셸 — 구독 원천이 `self.subs` 를 낸다.
        fn attach(&self) {
            self.register_subs();
            self.attach_without_subs();
        }

        fn attach_without_subs(&self) {
            self.coordinator.attach(RestorePorts {
                windows: self.windows.clone(),
                events: self.events.clone(),
                subs: self.slot.clone(),
                themes: self.themes.clone(),
            });
        }

        fn register_subs(&self) {
            *self.slot.subs.lock().unwrap() = Some(self.subs.clone());
        }

        /// 런타임 팝아웃 하나를 띄운다(같은 발급기) — 그 label · 모르는 내용 슬롯을 쥔 View.
        fn live_popout(&self) -> (String, Uuid) {
            let label = LabelSource::next_label(self.labels.as_ref());
            let mut mgr = self.layout.0.lock().unwrap();
            let view = mgr.create_window(&label).unwrap();
            let slot = first_slot(&mgr, view);
            mgr.set_unknown_content(view, slot, foreign()).unwrap();
            (label, view)
        }

        fn fingerprint(&self) -> (u64, u64, Vec<String>, usize) {
            let mgr = self.layout.0.lock().unwrap();
            let mut labels = mgr.list_windows();
            labels.sort();
            (mgr.version, mgr.attrs_rev(), labels, mgr.views.len())
        }

        fn windows_in_model(&self) -> Vec<String> {
            let mut labels = self.layout.0.lock().unwrap().list_windows();
            labels.sort();
            labels
        }

        fn ports_called_with_the_lock_free(&self) -> bool {
            self.windows
                .lock_was_free
                .lock()
                .unwrap()
                .iter()
                .chain(self.slot.lookups_with_lock_free.lock().unwrap().iter())
                .all(|free| *free)
        }
    }

    #[test]
    fn an_accept_swaps_the_screen_and_answers() {
        let rig = Rig::new(None);
        rig.attach();
        let (live_label, live_view) = rig.live_popout();
        assert_eq!(live_label, "slot-popup-1");
        let (version, attrs_rev) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.attrs_rev())
        };
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        let reply = rig.coordinator.answer(true).expect("수락");

        assert_eq!(
            reply,
            AnswerReply {
                restored_windows: 2,
                durable: false,
            },
            "main + 팝아웃 하나 · 기록기가 없다"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            let mut labels = mgr.list_windows();
            labels.sort();
            assert_eq!(
                labels,
                [MAIN_WINDOW_LABEL, "slot-popup-2"],
                "옛 팝아웃은 지우고 새 팝아웃은 떠 있는 창과 겹치지 않는 label 을 받는다"
            );
            assert_eq!(
                (mgr.version, mgr.attrs_rev()),
                (version + 1, attrs_rev + 1),
                "번호는 지금 값에서 하나씩 오른다"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_eq!(mgr.views[&main.active].name, "지난 탭");
            assert_eq!(main.attrs.bounds, Some(rect(100.0, 80.0, 1200.0, 800.0)));
            assert!(!mgr.views.contains_key(&live_view));
            let popout = &mgr.windows["slot-popup-2"];
            assert!(popout.attrs.maximized);
            let view = popout.active;
            assert_eq!(
                mgr.unknown_content(view, first_slot(&mgr, view)),
                Some(&foreign()),
                "모르는 내용 원문이 View 와 함께 온다"
            );
        }

        assert_eq!(
            rig.windows.calls(),
            [
                "focused?",
                "monitors",
                "defer_maximize slot-popup-2 in_model=false",
                "open_hidden slot-popup-2 at=false in_model=false",
                "place_main at=true maximized=false",
                "record main",
                "visible? main",
                "visible? slot-popup-2",
                "show slot-popup-2 in_model=true",
                "maximize slot-popup-2 (deferred)",
                "record slot-popup-2",
                "destroy slot-popup-1 in_model=false",
                "focused?",
                "focus main",
            ],
            "포커스 읽기 → 숨긴 채 만들고(최대화는 만들기 전에 미룬다 · 모델 밖 · 안 걸치는 자리는 버림) → 커밋 → 자리 → 지금 자리 적기 → 보이기(미룬 최대화) → 옛 창 거두기 → 포커스 다시 읽기 → main 에 포커스"
        );
        assert!(rig.windows.deferred().is_empty(), "보인 창의 미룸은 거뒀다");
        assert!(
            rig.ports_called_with_the_lock_free(),
            "창 포트 · 구독 원천은 락 밖에서 부른다"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1],
            "재계산은 커밋과 같은 임계구역"
        );
        assert_eq!(
            *rig.events.first_seen.lock().unwrap(),
            Some(true),
            "알림은 락 밖"
        );
        let tabs = rig.events.tabs.lock().unwrap().clone();
        assert_eq!(
            tabs,
            [
                (MAIN_WINDOW_LABEL.to_string(), version + 1),
                ("slot-popup-2".to_string(), version + 1)
            ]
        );
        assert_eq!(
            rig.events.layouts.lock().unwrap().len(),
            2,
            "탭마다 레이아웃"
        );

        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
        assert!(matches!(
            rig.coordinator.answer(true),
            Err(AnswerError::Conflict(AnswerConflict::NotAwaiting(
                CrashCopyStatus::Answered
            )))
        ));
    }

    /// ★수락은 사본의 창 테마를 민다★ — main 은 이미 떠 있고, 새 팝아웃은 모델에 들기 전에 첫 값을 당겼을 수 있다
    /// (그때는 전역 값). 보이기 앞에 · 레이아웃 락 밖에서 민다.
    #[test]
    fn an_accept_pushes_the_copys_window_themes() {
        let rig = Rig::new(None);
        rig.attach();
        let mut prev = ViewManager::new();
        prev.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        prev.create_window("slot-popup-70").unwrap();
        prev.set_window_theme("slot-popup-70", Some(UiTheme::EInk))
            .unwrap();
        prev.create_window("slot-popup-71").unwrap();
        rig.service.set_boot(
            Some(copy_of(to_persisted(&prev))),
            StateFileStatus::Ok,
            true,
        );

        rig.coordinator.answer(true).expect("수락");

        let mut sent = rig.theme_screen.sent.lock().unwrap().clone();
        sent.sort();
        let mut themes: Vec<String> = sent.iter().map(|(_, theme)| theme.clone()).collect();
        themes.sort();
        assert_eq!(sent.len(), 3, "main · 새 팝아웃 둘: {sent:?}");
        assert!(sent.contains(&(MAIN_WINDOW_LABEL.to_string(), "light".to_string())));
        assert_eq!(
            themes,
            ["dark", "e-ink", "light"],
            "새 팝아웃 하나는 사본의 테마, 하나는 전역 값: {sent:?}"
        );
        assert!(
            rig.theme_screen
                .lock_was_free
                .lock()
                .unwrap()
                .iter()
                .all(|free| *free),
            "레이아웃 락 밖에서 민다"
        );
    }

    /// 수락 전에 실패하면 화면이 그대로라 밀 것이 없다.
    #[test]
    fn a_failed_accept_pushes_no_theme() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-1".to_string());

        assert!(rig.coordinator.answer(true).is_err());

        assert!(rig.theme_screen.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn restored_popouts_stay_hidden_while_main_is_hidden_and_main_follows_the_copy_maximize() {
        let rig = Rig::new(None);
        rig.attach();
        rig.windows
            .hidden
            .lock()
            .unwrap()
            .push(MAIN_WINDOW_LABEL.into());
        let windows = previous_screen(2, true);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        let reply = rig.coordinator.answer(true).expect("수락");

        assert_eq!(reply.restored_windows, 3);
        // 사본의 팝아웃 차례는 영속 id 순이라 어느 새 label 이 최대화 팝아웃을 받는지는 매번 다르다 — 모델에서 읽는다.
        let maximized = {
            let mgr = rig.layout.0.lock().unwrap();
            assert!(
                mgr.windows[MAIN_WINDOW_LABEL].attrs.maximized,
                "모델은 사본의 main 최대화를 쥔다"
            );
            ["slot-popup-1", "slot-popup-2"].map(|label| mgr.windows[label].attrs.maximized)
        };
        assert_eq!(
            maximized.iter().filter(|m| **m).count(),
            1,
            "숨긴 팝아웃의 최대화도 사본대로"
        );
        let labels = ["slot-popup-1", "slot-popup-2"];
        let mut expected = vec!["focused?".to_string(), "monitors".to_string()];
        for (label, maximized) in labels.iter().zip(maximized) {
            if maximized {
                expected.push(format!("defer_maximize {label} in_model=false"));
            }
            expected.push(format!("open_hidden {label} at=false in_model=false"));
        }
        expected.extend(
            [
                "place_main at=true maximized=true",
                "record main",
                "visible? main",
            ]
            .map(String::from),
        );
        for (label, maximized) in labels.iter().zip(maximized) {
            expected.push(format!("visible? {label}"));
            expected.push(format!("hide {label} in_model=true"));
            if !maximized {
                expected.push(format!("record {label}"));
            }
        }
        assert_eq!(
            rig.windows.calls(),
            expected,
            "숨은 main 곁에 팝아웃을 띄우지 않는다 — 숨긴 최대화 팝아웃은 최대화를 미루고 기록도 안 한다(보였다 숨지 않게 · 사본의 최대화를 지우지 않게) · 보인 창이 없으니 포커스도 안 옮긴다"
        );
        let pending: Vec<String> = labels
            .iter()
            .zip(maximized)
            .filter(|(_, maximized)| *maximized)
            .map(|(label, _)| label.to_string())
            .collect();
        assert_eq!(
            rig.windows.deferred(),
            pending,
            "미룬 최대화는 트레이 「보이기」가 입힐 때까지 남는다"
        );
    }

    #[test]
    fn a_popout_window_gone_before_the_commit_is_dropped_from_the_model() {
        let rig = Rig::new(None);
        rig.attach();
        rig.live_popout();
        let version = rig.layout.0.lock().unwrap().version;
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        rig.windows.gone.lock().unwrap().push("slot-popup-2".into());

        let reply = rig.coordinator.answer(true).expect("수락은 커밋됐다");

        assert_eq!(reply.restored_windows, 1, "사라진 팝아웃은 세지 않는다");
        assert_eq!(rig.windows_in_model(), [MAIN_WINDOW_LABEL]);
        let calls = rig.windows.calls();
        assert!(
            calls.ends_with(&[
                "visible? main".to_string(),
                "visible? slot-popup-2".to_string(),
                "destroy slot-popup-1 in_model=false".to_string(),
            ]),
            "없는 창은 보이지도 적지도 않는다: {calls:?}"
        );
        assert_eq!(
            *rig.subs.seen.lock().unwrap(),
            [version + 1, version + 2],
            "지운 뒤 같은 락 안에서 다시 재계산한다"
        );
        assert!(rig.ports_called_with_the_lock_free());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
    }

    #[test]
    fn subscriptions_registered_after_the_ports_were_attached_are_used_at_commit() {
        let rig = Rig::new(None);
        rig.attach_without_subs();
        rig.register_subs();
        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

        rig.coordinator.answer(true).expect("수락");

        assert_eq!(
            rig.subs.seen.lock().unwrap().len(),
            1,
            "포트를 꽂을 때가 아니라 커밋 때 찾는다"
        );
        assert!(
            !rig.windows
                .calls()
                .iter()
                .any(|call| call.starts_with("focus ")),
            "보인 팝아웃이 없으면 포커스가 안 옮았으니 main 을 끌어오지 않는다"
        );
    }

    #[test]
    fn main_takes_the_focus_after_an_accept_only_if_the_app_had_it_before_and_still_has_it() {
        // (답 전에 다른 앱이 앞 · ②–④ 사이에 사람이 다른 앱으로 옮겼다 → main 을 끌어오나 · 포커스 읽기 횟수)
        for (background, switched_away, takes_focus, samples) in [
            (false, false, true, 2),
            (true, false, false, 1),
            (false, true, false, 2),
        ] {
            let rig = Rig::new(None);
            rig.attach();
            *rig.windows.background.lock().unwrap() = background;
            if switched_away {
                // 보인 팝아웃도 다른 앱에게서 앞자리를 못 가져왔다(가짜의 보이기가 내리는 `background` 를 덮는다).
                *rig.windows.focus_samples.lock().unwrap() = vec![true, false];
            }
            let windows = previous_screen(1, false);
            rig.service
                .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);

            rig.coordinator.answer(true).expect("수락");

            let calls = rig.windows.calls();
            let case = format!("background={background} switched_away={switched_away}");
            assert_eq!(
                calls.first().map(String::as_str),
                Some("focused?"),
                "{case} — 창을 만지기 전에 읽는다: {calls:?}"
            );
            assert!(
                calls
                    .iter()
                    .any(|call| call == "show slot-popup-1 in_model=true"),
                "{case} — 보인 팝아웃이 있다(그 보이기가 가짜의 「다른 앱이 앞」을 내린다): {calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|call| *call == "focused?").count(),
                samples,
                "{case} — 답 전의 표본이 거짓이면 다시 읽지 않는다: {calls:?}"
            );
            assert_eq!(
                calls.iter().any(|call| call.starts_with("focus ")),
                takes_focus,
                "{case} — 다른 앱이 앞에 있었거나 지금 있으면 main 을 끌어오지 않는다(가짜 Alt 로 그 앱의 포커스를 뺏는다): {calls:?}"
            );
            if takes_focus {
                assert_eq!(
                    calls[calls.len() - 2..],
                    ["focused?", "focus main"],
                    "{case} — 다시 읽기는 포커스 주기 바로 앞이다"
                );
            }
            assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
        }
    }

    #[test]
    fn a_window_that_fails_to_open_undoes_the_ones_made_and_leaves_the_answer_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(2, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-2".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls_but_deferrals(),
            [
                "focused?",
                "monitors",
                "open_hidden slot-popup-1 at=false in_model=false",
                "open_hidden slot-popup-2 at=false in_model=false",
                "destroy slot-popup-1 in_model=false",
            ]
        );
        assert_eq!(rig.fingerprint(), before, "모델은 그대로");
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting],
            "되돌림도 awaiting 을 알린다"
        );

        *rig.windows.fail_open.lock().unwrap() = None;
        let reply = rig
            .coordinator
            .answer(true)
            .expect("표지가 풀려 다시 답할 수 있다");
        assert_eq!(reply.restored_windows, 3);
    }

    #[test]
    fn a_hidden_window_that_will_not_be_destroyed_is_tried_once_more_and_the_answer_stays_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(2, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        *rig.windows.fail_open.lock().unwrap() = Some("slot-popup-2".into());
        *rig.windows.fail_destroy.lock().unwrap() = Some("slot-popup-1".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls_but_deferrals()[4..],
            [
                "destroy slot-popup-1 in_model=false",
                "destroy slot-popup-1 in_model=false",
            ],
            "한 번 더 해 보고 그만둔다"
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
    }

    #[test]
    fn a_commit_the_model_refuses_destroys_the_hidden_windows_and_leaves_the_answer_open() {
        let rig = Rig::new(None);
        rig.attach();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let (version, views) = {
            let mgr = rig.layout.0.lock().unwrap();
            (mgr.version, mgr.views.len())
        };
        *rig.windows.squat_on_open.lock().unwrap() = Some("slot-popup-1".into());

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert_eq!(
            rig.windows.calls(),
            [
                "focused?",
                "monitors",
                "defer_maximize slot-popup-1 in_model=false",
                "open_hidden slot-popup-1 at=false in_model=false",
                "destroy slot-popup-1 in_model=true",
            ],
            "커밋이 거절되면 만든 창을 거둔다"
        );
        {
            let mgr = rig.layout.0.lock().unwrap();
            assert_eq!(
                (mgr.version, mgr.views.len()),
                (version + 1, views + 1),
                "끼어든 창 하나 말고는 그대로"
            );
            let main = &mgr.windows[MAIN_WINDOW_LABEL];
            assert_ne!(
                mgr.views[&main.active].name, "지난 탭",
                "main 은 사본으로 안 바뀌었다"
            );
        }
        assert!(rig.subs.seen.lock().unwrap().is_empty(), "커밋 전에 멈췄다");
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Awaiting]
        );
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
    }

    #[test]
    fn a_label_source_that_collides_with_a_live_popout_fails_before_any_window() {
        // 조율자만 따로 센다 — 떠 있는 `slot-popup-1` 과 같은 label 을 낸다.
        let rig = Rig::new(Some(Arc::new(PopupCounter::default())));
        rig.attach();
        rig.live_popout();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        let err = rig.coordinator.answer(true).unwrap_err();

        assert!(matches!(err, AnswerError::Internal(_)), "{err:?}");
        assert!(rig.windows.calls().is_empty(), "준비에서 멈춘다");
        assert_eq!(rig.fingerprint(), before);
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert!(rig.service.begin_answer().is_ok(), "표지가 풀렸다");
    }

    #[test]
    fn an_accept_before_the_ports_are_attached_is_refused_but_a_reject_is_not() {
        let rig = Rig::new(None);
        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        assert!(matches!(
            rig.coordinator.answer(true),
            Err(AnswerError::NotReady)
        ));
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Awaiting);
        assert_eq!(rig.fingerprint(), before);

        assert_eq!(
            rig.coordinator.answer(false).expect("거절"),
            AnswerReply {
                restored_windows: 0,
                durable: false,
            }
        );
        assert_eq!(rig.service.status().crash_copy, CrashCopyStatus::Answered);
    }

    #[test]
    fn a_reject_answers_without_touching_the_screen() {
        let rig = Rig::new(None);
        rig.attach();
        rig.live_popout();
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let before = rig.fingerprint();

        let reply = rig.coordinator.answer(false).expect("거절");

        assert_eq!(
            reply,
            AnswerReply {
                restored_windows: 0,
                durable: false,
            }
        );
        assert_eq!(rig.fingerprint(), before);
        assert!(rig.windows.calls().is_empty());
        assert!(rig.events.tabs.lock().unwrap().is_empty());
        assert_eq!(
            rig.notices.seen(),
            [CrashCopyStatus::Awaiting, CrashCopyStatus::Answered]
        );
    }

    #[test]
    fn an_answer_without_a_copy_or_while_another_is_in_flight_conflicts() {
        let rig = Rig::new(None);
        rig.attach();
        assert!(matches!(
            rig.coordinator.answer(false),
            Err(AnswerError::Conflict(AnswerConflict::NotAwaiting(
                CrashCopyStatus::None
            )))
        ));

        let windows = previous_screen(0, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        let held = rig.service.begin_answer().unwrap();
        for accept in [true, false] {
            assert!(matches!(
                rig.coordinator.answer(accept),
                Err(AnswerError::Conflict(AnswerConflict::InFlight))
            ));
        }
        assert!(rig.windows.calls().is_empty());
        drop(held);
        assert!(rig.coordinator.answer(false).is_ok());
    }

    #[test]
    fn the_coordinator_status_is_the_service_status() {
        let rig = Rig::new(None);
        let windows = previous_screen(1, false);
        rig.service
            .set_boot(Some(copy_of(windows)), StateFileStatus::Ok, true);
        assert_eq!(rig.coordinator.status(), rig.service.status());
    }
}
