//! 화면 상태의 실행 수명 — 부팅 단계 플러그인(TRD S21-storage §6-5 ⓪–⑥) · 기록기 시작(⑦) · 답을 디스크에 붙이기
//! (§6-7 ⑤) · 정상 종료(§6-6). 넷이 나눠 쥐는 칸이 [`StateSession`] 이고, 기록기의 스냅숏 원천이 [`LiveSource`] 다.
//!
//! - ★플러그인([`init`])은 단일 인스턴스 플러그인 바로 뒤에 등록한다★ — 플러그인 setup 은 `build()` 안에서 등록
//!   순서대로 돌고 단일 인스턴스 플러그인이 거기서 둘째 인스턴스를 끝낸다(§6-5 사실 ① · ③). 그 뒤라야 디스크를
//!   바꿔도 되고, 설정 창(main)은 그보다 늦게 만들어지므로 창이 처음 당기는 모델이 판정한 모델이다.
//! - ★setup 은 `Err` 를 돌려주지 않는다★ — `Err` 면 빌드가 멈춰 앱이 아예 안 뜬다(N7 · D8). 실패는 log 하고 계속한다.
//! - ★복원 서비스 상태는 ⑥ 한 곳에서 정한다(I5)★ — 창이 아직 없으므로 모든 창의 첫 `restore_status` 당기기가 정해진
//!   값을 본다. 사용자 setup 에서 정하면 창을 만드는 자리(⑧ — `state::placement::restore_windows`)보다 앞인지를 손으로
//!   지켜야 한다.
//! - ★셸 실행 잠금은 [`StateSession::shutdown`] 이 명시적으로 놓는다★ — 이벤트 루프는 끝나면 곧장
//!   `process::exit` 한다(tao `platform_impl/windows/event_loop.rs` `run`). managed state 도 이 칸도 drop 되지 않아,
//!   기대면 잠금은 OS 가 핸들을 거둘 때에야 풀리고 기다리는 새 인스턴스가 그만큼 더 막힌다(`lock.rs` 의 `Drop`).

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use engram_dashboard_base::logging;
use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Emitter, Runtime};

use super::boot::{
    self, BootModel, BootPlan, FsBootFiles, Guard, StateAside, CRASH_COPY_FILE, STATE_FILE,
};
use super::convert::{to_persisted, RestoreWarning, StateRevision};
use super::lock::{self, StateLock};
use super::restore::{
    CrashCopy, CrashCopyStatus, RestoreNotifier, RestoreService, StateFileStatus,
    EVT_RESTORE_CHANGED,
};
use super::saver::{self, Clock, CloseFlag, RequestOutcome, SaveOutcome, SaverHandle, SystemClock};
use super::schema::WindowEntry;
use crate::discovery::DataLayout;
use crate::layout::{LabelSource, LayoutState, ViewManager, MAIN_WINDOW_LABEL};

const PLUGIN_NAME: &str = "engram-state-boot";

/// 부팅 단계가 채우는 것 — 빌더에서 manage 하는 것과 같은 인스턴스를 넘긴다(ADR-0102 — 첫 invoke 전에 있다).
pub struct Boot {
    pub layout: LayoutState,
    /// 복원하는 팝아웃의 label — 런타임 팝아웃과 같은 발급기여야 label 이 겹치지 않는다(§6-3).
    pub labels: Arc<dyn LabelSource>,
    pub session: Arc<StateSession>,
    /// ⑥ 이 상태를 정한다. 알림은 플러그인 setup 이 부팅 단계 앞에 꽂는다 — ⑥ 의 결정부터 알린다.
    pub restore: Arc<RestoreService>,
}

pub fn init<R: Runtime>(boot: Boot) -> TauriPlugin<R> {
    PluginBuilder::new(PLUGIN_NAME)
        .setup(move |app, _api| {
            boot.restore
                .set_notifier(Arc::new(TauriRestoreNotifier { app: app.clone() }));
            boot.run();
            Ok(())
        })
        .build()
}

/// 복원 상태가 바뀌면 main 창에 [`EVT_RESTORE_CHANGED`] 를 낸다. ⑥ 에는 창이 없어 받는 쪽이 없다 — 창은 첫
/// `restore_status` 당기기로 정해진 값을 본다(I5).
struct TauriRestoreNotifier<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> RestoreNotifier for TauriRestoreNotifier<R> {
    fn changed(&self, status: CrashCopyStatus) {
        if let Err(e) = self
            .app
            .emit_to(MAIN_WINDOW_LABEL, EVT_RESTORE_CHANGED, status)
        {
            tracing::warn!(
                module = "state",
                ?status,
                error = %e,
                "복원 상태 변경을 main 창에 알리지 못했다"
            );
        }
    }
}

impl Boot {
    fn run(self) {
        let paths = DataLayout::resolve();
        // ⓪ 데몬과 **다른 파일**(`app-*.log`)에 쓴다 — 한 파일을 두 프로세스가 나눠 쓰면 줄이 섞인다. 폴더는 데몬과
        //   같은 데이터 폴더라 기동 실패를 쫓을 때 두 로그가 한자리에 모인다.
        let log_file = logging::init_logging_with_file(&paths.logs_dir(), logging::LogKind::App);
        // 1차 폴더를 못 쓰면 로그 경로가 `%TEMP%` 아래로 갈릴 수 있어, 반환값 말고는 어디에 쓰고 있는지 아는 수단이
        //   없다(데몬 `run()` 의 "데이터 폴더 결정" 과 같은 이유).
        tracing::info!(
            data_dir = %paths.root().display(),
            log_file = ?log_file,
            "앱 로그 파일 결정"
        );
        self.run_steps(&paths.shell_run_dir(), &paths.shell_state_dir());
    }

    /// 부팅 단계 ①–⑥. 기록기는 띄우지 않는다 — 띄울 재료를 [`StateSession`] 에 맡기고 ⑦ 이 꺼낸다.
    fn run_steps(&self, run_dir: &Path, state_dir: &Path) {
        self.session.hold_lock(lock::acquire(run_dir));

        let files = FsBootFiles::in_dir(state_dir);
        let plan = boot::prepare(&files);

        let model = match &plan.model {
            BootModel::Default => ViewManager::new(),
            BootModel::Restore(file) => {
                let restored =
                    ViewManager::from_persisted(file.windows.clone(), self.labels.as_ref());
                restored.warnings.iter().for_each(report_restore_warning);
                restored.layout
            }
        };
        let marker =
            boot::write_run_marker(&files, &plan, to_persisted(&model), SystemClock.wall_ms());

        let revision = {
            // 이 락을 부팅 단계보다 먼저 잡는 쪽이 없어 독이 들 수 없다 — 들었어도 모델을 통째로 갈아끼운다.
            let mut layout = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
            *layout = model;
            StateRevision::of(&layout)
        };

        let state_file = state_file_status(&plan);
        let BootPlan {
            model,
            crash_copy,
            carry_resolved,
            guard,
            ..
        } = plan;
        // 가드 계약(§6-5 ③) — 이번 실행이 저장한다 ≡ 가드가 아니다. 가드면 기록기가 없어 답이 디스크에 붙지 않는다 —
        //   가드 ⅱ 의 원천은 판정이 실은 메모리의 원문이다(L2).
        let saves = guard.is_none();
        let ask = crash_copy.is_some();
        self.restore.set_boot(
            crash_copy.map(|copy| CrashCopy {
                text: copy.text,
                hash: copy.hash,
                file: copy.file,
            }),
            state_file,
            saves,
        );

        let saver = saves.then(|| PendingSaver {
            source: LiveSource::new(self.layout.clone()),
            files: saver::Fs::new(state_dir.join(STATE_FILE), state_dir.join(CRASH_COPY_FILE)),
            revision,
            carry_resolved,
        });
        tracing::info!(
            module = "state",
            restore = matches!(model, BootModel::Restore(_)),
            ask,
            state_file = state_file.as_wire(),
            marker = ?marker,
            saver = saver.is_some(),
            "부팅 단계 — 모델 · 복원 상태를 정했다"
        );
        self.session.cell().saver_start = saver;
    }
}

// TRD S21-storage §6-5 — 판정(③)과 동작(④)이 끝난 계획에서 읽는다.
fn state_file_status(plan: &BootPlan) -> StateFileStatus {
    if matches!(plan.guard, Some(Guard::StateUnreadable(_))) {
        return StateFileStatus::Unreadable;
    }
    match plan.state_aside {
        Some(StateAside::CopiedAside) => StateFileStatus::CorruptCopiedAside,
        Some(StateAside::NotCopied) => StateFileStatus::CorruptNotCopied,
        None => StateFileStatus::Ok,
    }
}

pub(super) fn report_restore_warning(warning: &RestoreWarning) {
    match warning {
        RestoreWarning::Internal(_) => {
            tracing::error!(module = "state", %warning, "화면 상태 복원")
        }
        _ => tracing::warn!(module = "state", %warning, "화면 상태 복원"),
    }
}

/// 부팅 단계 · 사용자 setup · `RunEvent::Exit` 가 나눠 쥐는 칸. 락은 잎이다 — 쥔 채 다른 락을 잡거나 기다리지 않는다
/// (`settled` 의 기다림은 칸 락을 놓고 선다).
#[derive(Default)]
pub struct StateSession {
    cell: Mutex<SessionCell>,
    /// 띄우는 중(`starting`)이 끝나면(실음 · 못 띄움 · 종료) 깨운다 — 답이 그 끝을 기다린다
    /// ([`StateSession::resolve_crash_copy`]).
    settled: Condvar,
}

#[derive(Default)]
struct SessionCell {
    lock: Option<StateLock>,
    /// 부팅 단계 ⑥ 이 세우고 ⑦([`StateSession::start_saver`])이 꺼낸다. `None` = 가드(N3 — 이 실행은 저장하지
    /// 않는다) · 이미 꺼냈다.
    saver_start: Option<PendingSaver>,
    /// 재료를 꺼낸 뒤 칸에 싣기 전까지의 기록기 닫힘 표지 — 그 사이 [`StateSession::shutdown`] 은 셸 실행 잠금을
    /// 놓기 전에 이것을 세운다. 안 세우면 실리지 않은 기록기가 잠금을 놓은 뒤에 써서, 그사이 관문을 지난 새
    /// 인스턴스의 파일을 덮는다.
    starting: Option<CloseFlag>,
    saver: Option<SaverHandle>,
    /// [`StateSession::shutdown`] 가 이미 돌았다.
    closed: bool,
}

struct PendingSaver {
    source: LiveSource,
    files: saver::Fs,
    /// 실행 표식(⑤)이 담은 모델의 변경 번호.
    revision: StateRevision,
    /// 실행 표식(⑤)이 실은 답한 사본의 해시 — 기록기가 이어 싣고 다시 지운다.
    carry_resolved: Option<String>,
}

// [`StateSession::published_saver`] 의 답.
enum Published {
    Ready(SaverHandle),
    /// 가드 · 못 띄움 · 이미 끝남.
    Absent,
    /// 다른 스레드가 띄우는 중인 채 마감이 지났다.
    StillStarting,
}

/// [`StateSession::resolve_crash_copy`] 의 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveResult {
    /// 그 해시를 실은 `state.json` 이 마감 안에 발행됐다 — 사본 지우기의 성패는 담지 않는다.
    Durable,
    /// 디스크에 붙지 않았다 — 기록기 없음(가드 · 못 띄움 · 이미 끝남 · 띄우는 다른 스레드가 마감 안에 싣지
    /// 못함) · 쓰기 실패 · 닫힘 · 마감 초과.
    NotDurable,
}

impl StateSession {
    fn cell(&self) -> MutexGuard<'_, SessionCell> {
        // 칸이 `Option` 넷과 깃발 하나뿐이라 반쯤 바뀐 상태가 없다 — 종료 경로가 잠금을 놓지 못하는 것이 더 나쁘다.
        self.cell.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hold_lock(&self, lock: Option<StateLock>) {
        self.cell().lock = lock;
    }

    /// 부팅 단계 ⑦ — 가드였거나 이미 띄웠으면(⑦ 전에 온 답이 먼저 띄운 경우 포함 —
    /// [`Self::resolve_crash_copy`]) 아무것도 하지 않는다.
    pub fn start_saver(&self) {
        let Some((pending, closed)) = self.take_pending() else {
            return;
        };
        match saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            pending.carry_resolved,
            closed,
        ) {
            Ok(handle) => self.publish_saver(handle),
            Err(e) => {
                self.end_start(|_| {});
                tracing::error!(
                    module = "state",
                    error = %e,
                    "기록기 스레드를 띄우지 못했다 — 이 실행은 화면 상태를 더 저장하지 않는다"
                );
            }
        }
    }

    // 띄울 재료를 꺼내고 그 기록기의 닫힘 표지를 칸에 「띄우는 중」으로 세운다 — 꺼낸 쪽은 반드시
    // [`Self::publish_saver`] 나 [`Self::end_start`] 로 끝맺는다(안 끝맺으면 답이 마감까지 기다린다).
    fn take_pending(&self) -> Option<(PendingSaver, CloseFlag)> {
        let mut cell = self.cell();
        let pending = cell.saver_start.take()?;
        let closed = CloseFlag::default();
        cell.starting = Some(closed.clone());
        Some((pending, closed))
    }

    // 「띄우는 중」을 거두고 기다리는 답을 깨운다 — `then` 은 칸 락 안에서 돈다(칸 대입만 할 것).
    fn end_start<T>(&self, then: impl FnOnce(&mut SessionCell) -> T) -> T {
        let out = {
            let mut cell = self.cell();
            cell.starting = None;
            then(&mut cell)
        };
        self.settled.notify_all();
        out
    }

    // 띄운 기록기를 칸에 싣는다 — 띄우는 동안(락 밖) [`Self::shutdown`] 이 돌았으면 싣지 않는다(그 기록기는 종료가
    // 이미 닫았다 — `starting`).
    fn publish_saver(&self, handle: SaverHandle) {
        let published = self.end_start(|cell| {
            if cell.closed {
                false
            } else {
                cell.saver = Some(handle.clone());
                true
            }
        });
        if !published {
            handle.close();
            tracing::info!(
                module = "state",
                "기록기를 띄우는 사이 종료가 돌아 그 기록기를 닫는다 — 아무것도 쓰지 않는다"
            );
        }
    }

    /// 답한 사본의 해시를 기록기에 넘기고(`Resolve`) 마감([`saver::REPLY_DEADLINE`])까지 기다린다(TRD §6-7 ⑤).
    ///
    /// ★기록기가 아직 없으면 먼저 ⑦([`Self::start_saver`])을 부른다★ — 답이 ⑦ 보다 먼저 와도 곧 뜰 기록기를 두고
    /// `NotDurable` 이 되지 않게 한다(오늘의 부팅 차례로는 오지 않는다 — 창은 ⑦ 뒤 ⑧ 에서 생긴다). 가드 · 이미 띄움 ·
    /// 종료 뒤면 그 부름은 아무것도 하지 않는다.
    ///
    /// ★다른 스레드(⑦ · 다른 답)가 띄우는 중이면 싣거나 실패할 때까지 기다린다★ — 그 기다림과 `Resolve` 답 기다림이
    /// 같은 마감 하나를 나눠 쓴다. 안 기다리면 그 순간엔 기록기가 없는 것으로 보여 답이 디스크에 붙지 않는다.
    ///
    /// ★아무 락도 쥐지 않고 기다린다 — 부르는 쪽도 레이아웃 락을 쥔 채 부르지 않는다★: 기록기는
    /// 스냅숏을 뜨려고 그 락을 잡는다(`saver::SnapshotSource`). 쥐면 답이 늘 마감을 넘긴다.
    pub fn resolve_crash_copy(&self, hash: String) -> ResolveResult {
        self.resolve_within(hash, saver::REPLY_DEADLINE)
    }

    fn resolve_within(&self, hash: String, deadline: Duration) -> ResolveResult {
        let until = Instant::now() + deadline;
        let started = self.cell().saver.is_some();
        if !started {
            self.start_saver();
        }
        let saver = match self.published_saver(until) {
            Published::Ready(saver) => saver,
            Published::Absent => {
                tracing::info!(
                    module = "state",
                    hash = %hash,
                    "기록기가 없어 답을 디스크에 붙이지 않는다"
                );
                return ResolveResult::NotDurable;
            }
            Published::StillStarting => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "기록기가 마감 안에 뜨지 않아 답을 디스크에 붙이지 않는다"
                );
                return ResolveResult::NotDurable;
            }
        };
        let left = until.saturating_duration_since(Instant::now());
        match saver.resolve(hash.clone(), left) {
            RequestOutcome::Done(SaveOutcome::Written) => ResolveResult::Durable,
            outcome => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    ?outcome,
                    "답을 마감 안에 디스크에 붙이지 못했다"
                );
                ResolveResult::NotDurable
            }
        }
    }

    // 띄우는 중이면 그 끝(실음 · 못 띄움 · 종료)을 `until` 까지 기다려 실린 기록기를 돌려준다. 칸 락은 기다리는
    // 동안 놓인다 — 띄우는 쪽이 실으려면 그 락을 잡아야 한다. 로그는 부르는 쪽이 남긴다(갈래마다 한 줄).
    fn published_saver(&self, until: Instant) -> Published {
        let mut cell = self.cell();
        while cell.starting.is_some() && !cell.closed {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Published::StillStarting;
            }
            cell = self
                .settled
                .wait_timeout(cell, left)
                .map_or_else(|poisoned| poisoned.into_inner().0, |(cell, _)| cell);
        }
        match cell.saver.clone() {
            Some(saver) => Published::Ready(saver),
            None => Published::Absent,
        }
    }

    /// `RunEvent::Exit` — 정상 종료 쓰기(`Final`)를 마감([`saver::REPLY_DEADLINE`])까지 기다린 뒤 셸 실행 잠금을
    /// 놓는다(§6-6). 기록기가 없으면(가드 · 못 띄움) 잠금만 놓는다(N7). 두 번째 부름은 아무것도 하지 않는다.
    ///
    /// ★메인 스레드에서 불린다 — 아무 락도 쥐지 않고 기다린다★: 기록기는 스냅숏을 뜨려고 레이아웃 락을 잡는다
    /// (`saver::SnapshotSource`).
    pub fn shutdown(&self) {
        let (saver, lock) = {
            let mut cell = self.cell();
            if std::mem::replace(&mut cell.closed, true) {
                return;
            }
            cell.saver_start = None;
            // 잠금을 놓기 전에 — 띄우는 중인 기록기는 아직 실리지 않아 아래 `Final` 이 닿지 않는다(칸 머리
            //   `starting`).
            if let Some(starting) = cell.starting.take() {
                starting.close();
            }
            (cell.saver.take(), cell.lock.take())
        };
        self.settled.notify_all();
        match saver.map(|saver| saver.finish(saver::REPLY_DEADLINE)) {
            Some(RequestOutcome::Done(SaveOutcome::Written)) => {
                tracing::info!(module = "state", "정상 종료를 적었다")
            }
            // 마감을 넘긴 것은 `finish` 가 이미 warn 했다.
            Some(RequestOutcome::TimedOut) => {}
            Some(outcome) => tracing::warn!(
                module = "state",
                ?outcome,
                "정상 종료를 적지 못했다 — 다음 부팅은 비정상 종료로 읽는다"
            ),
            None => tracing::info!(module = "state", "기록기가 없어 정상 종료 쓰기 없이 끝난다"),
        }
        drop(lock);
    }
}

/// 기록기의 스냅숏 원천 — 레이아웃 모델. 창 게터는 부르지 않는다(위치 · 크기는 모델에 적힌 값을 읽는다).
pub struct LiveSource {
    layout: LayoutState,
}

impl LiveSource {
    pub fn new(layout: LayoutState) -> Self {
        Self { layout }
    }
}

impl saver::SnapshotSource for LiveSource {
    type Revision = StateRevision;

    fn revision(&self) -> StateRevision {
        // 번호 읽기는 반쯤 바뀐 모델에도 해롭지 않다 — 독 든 모델을 쓰지 않는 것은 `snapshot` 이 한다.
        let layout = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
        StateRevision::of(&layout)
    }

    fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
        // 독 든 모델은 쓰지 않는다 — 패닉한 변경이 반쯤 남았을 수 있고, 쓰면 다음 부팅이 그 모양을 복원한다.
        let layout = self
            .layout
            .0
            .lock()
            .map_err(|_| "레이아웃 락에 독이 들었다".to_string())?;
        Ok(to_persisted(&layout))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::path::PathBuf;

    use super::*;
    use crate::commands::popout::PopupCounter;
    use crate::layout::{LayoutEvents, ViewSnapshot, WindowBounds, WindowTabsPayload};
    use crate::state::codec;
    use crate::state::lock::LOCK_FILE;
    use crate::state::placement::{Landing, MonitorArea};
    use crate::state::restore::{
        AnswerEnd, RestoreCoordinator, RestorePorts, RestoreStatusView, RestoreWindows,
        SubscriptionSource,
    };
    use crate::state::saver::SnapshotSource;
    use crate::state::schema::{StateFile, STATE_VERSION};
    use crate::theme::{EffectiveThemes, ThemeControl, ThemeWindows, UiSettingsPayload, UiTheme};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-state-boot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("임시 폴더");
        dir
    }

    fn source() -> (LiveSource, LayoutState) {
        let layout = LayoutState::new();
        (LiveSource::new(layout.clone()), layout)
    }

    fn main_view(layout: &LayoutState) -> uuid::Uuid {
        layout.0.lock().unwrap().windows[MAIN_WINDOW_LABEL].active
    }

    fn read_state(dir: &Path) -> StateFile {
        let text = std::fs::read_to_string(dir.join(STATE_FILE)).expect("state.json");
        codec::decode(&text).expect("상태 파일").0
    }

    fn lock_is_free(run_dir: &Path) -> bool {
        let file = OpenOptions::new()
            .write(true)
            .open(run_dir.join(LOCK_FILE))
            .expect("잠금 파일");
        let free = file.try_lock().is_ok();
        if free {
            file.unlock().expect("잡은 잠금을 푼다");
        }
        free
    }

    fn boot(layout: &LayoutState) -> (Boot, Arc<StateSession>) {
        let session = Arc::new(StateSession::default());
        let boot = Boot {
            layout: layout.clone(),
            labels: Arc::new(PopupCounter::default()),
            session: session.clone(),
            restore: Arc::new(RestoreService::new()),
        };
        (boot, session)
    }

    #[derive(Default)]
    struct Notices(Mutex<Vec<CrashCopyStatus>>);

    impl RestoreNotifier for Notices {
        fn changed(&self, status: CrashCopyStatus) {
            self.0.lock().unwrap().push(status);
        }
    }

    impl Notices {
        fn seen(&self) -> Vec<CrashCopyStatus> {
            self.0.lock().unwrap().clone()
        }
    }

    fn listen(boot: &Boot) -> Arc<Notices> {
        let notices = Arc::new(Notices::default());
        boot.restore.set_notifier(notices.clone());
        notices
    }

    /// 탭 이름 하나를 바꾼 화면 — 앞 실행이 남긴 `state.json` 원문과 그 탭.
    fn previous_run(clean_exit: bool, resolved: Option<String>) -> (String, uuid::Uuid) {
        let mut previous = ViewManager::new();
        let view = previous.windows[MAIN_WINDOW_LABEL].active;
        previous.rename_tab(view, "지난 탭".into()).unwrap();
        let text = codec::encode(&StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1,
            clean_exit,
            resolved_crash_copy: resolved,
            windows: to_persisted(&previous),
        })
        .unwrap();
        (text, view)
    }

    // ── 스냅숏 원천 ──

    #[test]
    fn revision_moves_with_layout_and_attrs() {
        let (source, layout) = source();
        let start = source.revision();
        assert_eq!(source.revision(), start, "바뀐 것이 없으면 같다");

        let view = main_view(&layout);
        layout
            .0
            .lock()
            .unwrap()
            .rename_tab(view, "새 이름".into())
            .unwrap();
        let after_layout = source.revision();
        assert_ne!(after_layout.layout, start.layout);

        layout
            .0
            .lock()
            .unwrap()
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        let after_attrs = source.revision();
        assert_ne!(after_attrs.attrs, after_layout.attrs);
        assert_eq!(after_attrs.layout, after_layout.layout);
    }

    #[test]
    fn snapshot_carries_the_model() {
        let (source, layout) = source();
        let view = main_view(&layout);
        layout
            .0
            .lock()
            .unwrap()
            .rename_tab(view, "저장할 탭".into())
            .unwrap();
        layout
            .0
            .lock()
            .unwrap()
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();

        let windows = source.snapshot().expect("스냅숏");
        let expected = to_persisted(&layout.0.lock().unwrap());
        assert_eq!(windows, expected);
    }

    #[test]
    fn a_poisoned_layout_lock_fails_the_snapshot_without_panicking() {
        let (source, layout) = source();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = layout.0.lock().unwrap();
            panic!("독");
        }));
        assert!(layout.0.is_poisoned());

        assert!(source.snapshot().is_err());
        let _ = source.revision();
    }

    // ── 부팅 단계 ①–⑥ · ⑦ · 종료 ──

    #[test]
    fn a_clean_previous_run_is_restored_marked_and_closed_cleanly() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let saved_tab = {
            let mut previous = ViewManager::new();
            let view = previous.windows[MAIN_WINDOW_LABEL].active;
            previous.rename_tab(view, "지난 탭".into()).unwrap();
            previous.create_window("slot-popup-7").unwrap();
            previous
                .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Dark))
                .unwrap();
            let text = codec::encode(&StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1,
                clean_exit: true,
                resolved_crash_copy: None,
                windows: to_persisted(&previous),
            })
            .unwrap();
            std::fs::write(state_dir.join(STATE_FILE), text).unwrap();
            view
        };

        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        let notices = listen(&boot);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            notices.seen(),
            [CrashCopyStatus::None],
            "⑥ 이 한 번 정한다(I5)"
        );
        {
            let mgr = layout.0.lock().unwrap();
            assert_eq!(mgr.views[&saved_tab].name, "지난 탭");
            assert_eq!(mgr.windows.len(), 2, "main + 복원한 팝아웃");
            assert!(
                mgr.windows.contains_key("slot-popup-1"),
                "팝아웃은 이 실행의 발급기에서 새 label 을 받는다"
            );
            assert_eq!(
                mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme,
                Some(UiTheme::Dark)
            );
        }
        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit, "실행 표식");
        assert_eq!(marker.windows.len(), 2, "main · 팝아웃");
        assert!(!lock_is_free(&run_dir), "잠금은 종료까지 쥔다");

        session.start_saver();
        session.shutdown();
        assert!(read_state(&state_dir).clean_exit, "정상 종료 쓰기");
        assert!(lock_is_free(&run_dir), "종료가 잠금을 놓는다");
        session.shutdown();
    }

    #[test]
    fn a_first_boot_writes_the_default_model_as_the_marker() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state").join("absent");
        let layout = LayoutState::new();
        let (boot, _session) = boot(&layout);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            boot.restore.status().state_file,
            StateFileStatus::Ok,
            "없는 파일은 ok"
        );
        assert!(boot.restore.status().saves, "가드가 아니다 — 저장한다");
        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit);
        assert_eq!(
            marker.windows,
            to_persisted(&layout.0.lock().unwrap()),
            "표식이 담은 것이 메모리 모델이다"
        );
    }

    #[test]
    fn an_unreadable_state_file_guards_the_run_and_never_starts_a_saver() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        // 폴더는 「없음」도 「못 쓸 파일」도 아닌 읽기 IO 실패다(I3) — 가드.
        std::fs::create_dir(state_dir.join(STATE_FILE)).unwrap();
        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(boot.restore.status().crash_copy, CrashCopyStatus::None);
        assert_eq!(
            boot.restore.status().state_file,
            StateFileStatus::Unreadable,
            "가드 ⅰ — 사본이 없어도 ⑥ 이 정한다"
        );
        assert!(!boot.restore.status().saves, "가드 ⅰ — 저장하지 않는다");
        assert!(session.cell().saver_start.is_none(), "기록기 재료가 없다");
        assert!(
            state_dir.join(STATE_FILE).is_dir(),
            "실행 표식을 쓰지 않는다"
        );
        session.start_saver();
        assert!(
            session.cell().saver.is_none(),
            "⑦ 은 아무것도 띄우지 않는다"
        );

        session.shutdown();
        assert!(lock_is_free(&run_dir), "가드여도 종료가 잠금을 놓는다");
        assert!(state_dir.join(STATE_FILE).is_dir());
    }

    #[test]
    fn an_unusable_state_file_kept_aside_is_reported_from_the_first_status() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        std::fs::write(state_dir.join(STATE_FILE), "{broken").unwrap();
        let (boot, session) = boot(&LayoutState::new());
        let notices = listen(&boot);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            boot.restore.status().state_file,
            StateFileStatus::CorruptCopiedAside
        );
        assert!(
            boot.restore.status().saves,
            "못 쓸 파일은 가드가 아니다 — 저장한다"
        );
        assert_eq!(
            notices.seen(),
            [CrashCopyStatus::None],
            "사본이 없어도 ⑥ 이 정한다"
        );
        assert_eq!(
            std::fs::read_to_string(state_dir.join("state.json.corrupt")).unwrap(),
            "{broken"
        );
        session.shutdown();
    }

    #[test]
    fn an_unusable_state_file_that_could_not_be_kept_aside_is_reported_as_such() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        std::fs::write(state_dir.join(STATE_FILE), "{broken").unwrap();
        // 떠 둘 자리에 비지 않은 폴더가 있다 — 파일로 갈아끼우는 rename 이 실패한다.
        let blocker = state_dir.join("state.json.corrupt");
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("x"), "x").unwrap();
        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            boot.restore.status().state_file,
            StateFileStatus::CorruptNotCopied
        );
        assert!(
            boot.restore.status().saves,
            "떠 두기 실패도 가드가 아니다(D8)"
        );
        assert!(
            !read_state(&state_dir).clean_exit,
            "실행 표식이 원본을 갈아끼웠다(D8)"
        );
        session.shutdown();
    }

    #[test]
    fn the_state_file_status_reads_the_state_file_only() {
        let plan = |guard: Option<Guard>, state_aside: Option<StateAside>| BootPlan {
            model: BootModel::Default,
            actions: Vec::new(),
            crash_copy: None,
            carry_resolved: None,
            guard,
            state_aside,
        };
        let unreadable = || Some(Guard::StateUnreadable("잠김".into()));
        let not_written = || Some(Guard::CrashCopyNotWritten("새 판의 사본".into()));
        for (guard, aside, expected) in [
            (None, None, StateFileStatus::Ok),
            (unreadable(), None, StateFileStatus::Unreadable),
            (not_written(), None, StateFileStatus::Ok),
            (
                None,
                Some(StateAside::CopiedAside),
                StateFileStatus::CorruptCopiedAside,
            ),
            (
                None,
                Some(StateAside::NotCopied),
                StateFileStatus::CorruptNotCopied,
            ),
        ] {
            let label = format!("{guard:?} · {aside:?}");
            assert_eq!(state_file_status(&plan(guard, aside)), expected, "{label}");
        }
    }

    // ── 크래시 사본 · 복원 서비스(⑥) · 답을 디스크에 붙이기 ──

    #[test]
    fn an_unclean_previous_run_becomes_the_crash_copy_and_awaits_an_answer() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, saved_tab) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();

        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        let notices = listen(&boot);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            std::fs::read_to_string(state_dir.join(CRASH_COPY_FILE)).unwrap(),
            raw,
            "사본 = 앞 실행의 state.json 원문"
        );
        assert_eq!(
            notices.seen(),
            [CrashCopyStatus::Awaiting],
            "⑥ 이 한 번 정한다(I5)"
        );
        assert_eq!(boot.restore.status().windows, Some(1), "main 만");
        let status = boot.restore.status();
        assert!(status.saves, "가드가 아니다 — 이 실행이 저장한다");
        assert_eq!(status.durable, Some(true), "묻는 동안은 saves 와 같다");
        assert!(
            !layout.0.lock().unwrap().views.contains_key(&saved_tab),
            "묻는 동안은 기본 화면"
        );
        assert!(!read_state(&state_dir).clean_exit, "실행 표식");
        {
            let ticket = boot.restore.begin_answer().unwrap();
            assert_eq!(ticket.copy().text, raw);
        }

        session.start_saver();
        let hash = codec::crash_copy_hash(&raw);
        assert_eq!(
            session.resolve_crash_copy(hash.clone()),
            ResolveResult::Durable
        );
        assert!(
            !state_dir.join(CRASH_COPY_FILE).exists(),
            "답을 디스크에 붙인 뒤 사본을 지운다"
        );
        assert_eq!(read_state(&state_dir).resolved_crash_copy, Some(hash));
        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }

    #[test]
    fn an_unanswered_copy_survives_a_clean_exit_and_is_asked_again() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();
        {
            let (boot, session) = boot(&LayoutState::new());
            boot.run_steps(&run_dir, &state_dir);
            session.start_saver();
            session.shutdown();
        }
        assert!(read_state(&state_dir).clean_exit);

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(
            std::fs::read_to_string(state_dir.join(CRASH_COPY_FILE)).unwrap(),
            raw,
            "정상 종료는 사본을 지우지 않는다(D6)"
        );
        assert_eq!(boot.restore.status().crash_copy, CrashCopyStatus::Awaiting);
        session.shutdown();
    }

    #[test]
    fn a_crash_copy_that_cannot_be_read_guards_the_run_and_asks_from_memory() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();
        // 폴더는 「없음」도 「못 쓸 파일」도 아닌 읽기 IO 실패다(I3) — 덮어도 되는지 모른다.
        std::fs::create_dir(state_dir.join(CRASH_COPY_FILE)).unwrap();

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        let status = boot.restore.status();
        assert!(!status.saves, "가드 ⅱ — 이 실행은 저장하지 않는다");
        assert_eq!(status.durable, Some(false), "묻는 동안은 saves 와 같다");
        assert_eq!(
            status.state_file,
            StateFileStatus::Ok,
            "가드 ⅱ 는 state_file 에 싣지 않는다"
        );
        {
            let ticket = boot.restore.begin_answer().expect("가드 ⅱ 여도 묻는다");
            assert_eq!(
                ticket.copy().text,
                raw,
                "복원 원천 = 메모리의 state.json 원문(L2)"
            );
        }
        assert!(
            session.cell().saver_start.is_none(),
            "가드 — 기록기 재료가 없다"
        );
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            raw,
            "디스크의 state.json 바이트 그대로"
        );
        session.start_saver();
        assert_eq!(
            session.resolve_crash_copy(codec::crash_copy_hash(&raw)),
            ResolveResult::NotDurable
        );
        session.shutdown();
    }

    // 가드 ⅱ 는 `state_file` 이 `ok` 라 답한 뒤에 그 사실을 나르는 칸이 `saves` 하나다(가드 ⅱ 안내 — ADR-0276).
    #[test]
    fn a_run_that_could_not_write_the_crash_copy_still_says_so_after_the_answer() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();
        std::fs::create_dir(state_dir.join(CRASH_COPY_FILE)).unwrap();

        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        let coordinator = RestoreCoordinator::new(
            boot.restore.clone(),
            layout,
            session.clone(),
            boot.labels.clone(),
        );
        boot.run_steps(&run_dir, &state_dir);
        session.start_saver();

        let reply = coordinator.answer(false).expect("거절");

        assert!(!reply.durable, "기록기가 없다");
        assert_eq!(
            boot.restore.status(),
            RestoreStatusView {
                crash_copy: CrashCopyStatus::Answered,
                saved_at_ms: None,
                windows: None,
                tabs: None,
                durable: None,
                saves: false,
                state_file: StateFileStatus::Ok,
            }
        );
        session.shutdown();
    }

    #[test]
    fn an_unreadable_state_file_with_an_unanswered_copy_asks_without_durability() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(CRASH_COPY_FILE), &raw).unwrap();
        // 폴더는 「없음」도 「못 쓸 파일」도 아닌 읽기 IO 실패다(I3) — 가드 ⅰ.
        std::fs::create_dir(state_dir.join(STATE_FILE)).unwrap();

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        let status = boot.restore.status();
        assert_eq!(status.crash_copy, CrashCopyStatus::Awaiting);
        assert!(!status.saves);
        assert_eq!(status.durable, Some(false));
        assert_eq!(status.state_file, StateFileStatus::Unreadable);

        let ticket = boot.restore.begin_answer().unwrap();
        boot.restore.finish_answer(ticket, AnswerEnd::Answered);
        let answered = boot.restore.status();
        assert!(!answered.saves, "답한 뒤에도 그대로");
        assert_eq!(answered.state_file, StateFileStatus::Unreadable);
        session.shutdown();
    }

    #[test]
    fn an_answered_hash_rides_the_marker_and_the_saver_when_the_copy_cannot_be_read() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(true, Some("h".into()));
        std::fs::write(state_dir.join(STATE_FILE), raw).unwrap();
        std::fs::create_dir(state_dir.join(CRASH_COPY_FILE)).unwrap();

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(boot.restore.status().crash_copy, CrashCopyStatus::None);
        assert_eq!(read_state(&state_dir).resolved_crash_copy, Some("h".into()));
        assert_eq!(
            session
                .cell()
                .saver_start
                .as_ref()
                .and_then(|pending| pending.carry_resolved.clone()),
            Some("h".into())
        );
        session.shutdown();
    }

    #[test]
    fn an_answer_before_the_saver_step_starts_the_saver_and_sticks_to_disk() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);
        assert!(session.cell().saver.is_none(), "⑦ 전");

        let hash = codec::crash_copy_hash(&raw);
        assert_eq!(
            session.resolve_crash_copy(hash.clone()),
            ResolveResult::Durable
        );
        assert!(!state_dir.join(CRASH_COPY_FILE).exists());
        assert_eq!(read_state(&state_dir).resolved_crash_copy, Some(hash));
        assert!(
            session.cell().saver_start.is_none(),
            "⑦ 의 재료를 답이 썼다"
        );

        session.start_saver();
        session.shutdown();
        assert!(
            read_state(&state_dir).clean_exit,
            "답이 띄운 기록기가 정상 종료를 적는다"
        );
        assert!(lock_is_free(&run_dir));
    }

    #[test]
    fn resolving_without_a_saver_is_not_durable() {
        let session = StateSession::default();
        assert_eq!(
            session.resolve_crash_copy("h".into()),
            ResolveResult::NotDurable
        );
    }

    #[test]
    fn shutdown_without_a_saver_still_releases_the_lock() {
        let run_dir = temp_dir("run");
        let session = StateSession::default();
        session.hold_lock(lock::acquire(&run_dir));
        assert!(!lock_is_free(&run_dir));

        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }

    #[test]
    fn a_saver_started_while_shutdown_ran_is_closed_instead_of_published() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();
        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);
        let marker = std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap();

        // `start_saver` 의 틈을 손으로 벌린다 — 재료를 꺼내 락 밖에서 띄우는 사이 종료가 돈다.
        let handle = spawn_pending(&session);
        let witness = handle.clone();
        session.shutdown();
        assert!(lock_is_free(&run_dir), "종료가 잠금을 놓았다");
        // 싣기 전에 본다 — 잠금이 풀린 지금 이미 닫혀 있어야 한다(종료가 잠금보다 먼저 세웠다).
        assert_eq!(
            witness.resolve(codec::crash_copy_hash(&raw), saver::REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped),
            "잠금을 놓은 뒤 실리지 않은 기록기가 쓰지 않는다"
        );
        session.publish_saver(handle);

        assert!(session.cell().saver.is_none(), "닫힌 세션에 싣지 않는다");
        assert!(session.cell().starting.is_none());
        assert_eq!(
            session.resolve_crash_copy(codec::crash_copy_hash(&raw)),
            ResolveResult::NotDurable
        );
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            marker,
            "종료 뒤 state.json 은 그대로"
        );
        assert!(state_dir.join(CRASH_COPY_FILE).exists(), "사본도 그대로");
    }

    /// `start_saver` 의 앞 절반 — 재료를 꺼내 「띄우는 중」을 세우고 띄운다(싣지 않는다).
    fn spawn_pending(session: &StateSession) -> SaverHandle {
        let (pending, closed) = session.take_pending().expect("기록기 재료");
        saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            pending.carry_resolved,
            closed,
        )
        .expect("기록기 스레드")
    }

    /// 크래시 사본이 있는 부팅을 ⑥ 까지 돌린다 — 그 세션 · 잠금 폴더 · 상태 폴더 · 사본 해시.
    fn booted_with_copy() -> (Arc<StateSession>, PathBuf, PathBuf, String) {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, _) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();
        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);
        (session, run_dir, state_dir, codec::crash_copy_hash(&raw))
    }

    #[test]
    fn a_shutdown_between_taking_the_materials_and_spawning_closes_the_saver_first() {
        let (session, run_dir, state_dir, hash) = booted_with_copy();
        let marker = std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap();

        let (pending, closed) = session.take_pending().expect("기록기 재료");
        session.shutdown();
        assert!(lock_is_free(&run_dir));
        let handle = saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            pending.carry_resolved,
            closed,
        )
        .expect("기록기 스레드");

        assert_eq!(
            handle.resolve(hash, saver::REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped),
            "잠금을 놓은 뒤에 뜬 기록기는 처음부터 닫혀 있다"
        );
        session.publish_saver(handle);
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            marker
        );
    }

    #[test]
    fn an_answer_while_another_thread_starts_the_saver_waits_for_it_and_sticks_to_disk() {
        let (session, run_dir, state_dir, hash) = booted_with_copy();
        let handle = spawn_pending(&session);

        let answer = {
            let session = session.clone();
            let hash = hash.clone();
            std::thread::spawn(move || session.resolve_crash_copy(hash))
        };
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !answer.is_finished(),
            "띄우는 중이면 기록기가 없다고 끝내지 않고 기다린다"
        );
        session.publish_saver(handle);

        assert_eq!(answer.join().unwrap(), ResolveResult::Durable);
        assert_eq!(read_state(&state_dir).resolved_crash_copy, Some(hash));
        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }

    #[test]
    fn an_answer_gives_up_at_the_deadline_on_a_saver_that_never_lands() {
        let (session, _run_dir, _state_dir, hash) = booted_with_copy();
        let handle = spawn_pending(&session);

        let started = Instant::now();
        assert_eq!(
            session.resolve_within(hash, Duration::from_millis(100)),
            ResolveResult::NotDurable
        );
        assert!(started.elapsed() >= Duration::from_millis(100));

        session.publish_saver(handle);
        session.shutdown();
    }

    #[test]
    fn a_shutdown_wakes_an_answer_waiting_on_the_saver_start() {
        let (session, _run_dir, _state_dir, hash) = booted_with_copy();
        let handle = spawn_pending(&session);

        let answer = {
            let session = session.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                (session.resolve_crash_copy(hash), started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        session.shutdown();

        let (result, waited) = answer.join().unwrap();
        assert_eq!(result, ResolveResult::NotDurable);
        assert!(waited < saver::REPLY_DEADLINE, "마감까지 서 있지 않는다");
        session.publish_saver(handle);
    }

    // ── 복원 조율자 — 부팅 단계부터 답이 디스크에 붙기까지 ──

    struct NoWindows;

    impl RestoreWindows for NoWindows {
        fn app_has_focus(&self) -> bool {
            false
        }
        fn monitors(&self) -> Vec<MonitorArea> {
            Vec::new()
        }
        fn open_hidden(
            &self,
            _label: &str,
            _at: Option<(WindowBounds, Landing)>,
            _maximized: bool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn visibility(&self, _label: &str) -> Option<bool> {
            Some(true)
        }
        fn place_main(&self, _at: Option<(WindowBounds, Landing)>, _maximized: bool) {}
        fn set_shown(&self, _label: &str, _shown: bool) {}
        fn record_placement(&self, _label: &str) {}
        fn focus(&self, _label: &str) {}
        fn destroy(&self, _label: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct NoSubscriptions;

    impl SubscriptionSource for NoSubscriptions {
        fn current(&self) -> Option<Arc<dyn crate::layout::SubscriptionSync>> {
            None
        }
    }

    struct NoThemeWindows;

    impl ThemeWindows for NoThemeWindows {
        fn labels(&self) -> Vec<String> {
            Vec::new()
        }
        fn send(&self, _label: &str, _payload: UiSettingsPayload) -> Result<(), String> {
            Ok(())
        }
    }

    struct NoEvents;

    impl LayoutEvents for NoEvents {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
        fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
    }

    #[test]
    fn an_accepted_crash_copy_is_restored_and_sticks_to_disk() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let (raw, saved_tab) = previous_run(false, None);
        std::fs::write(state_dir.join(STATE_FILE), &raw).unwrap();

        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        let coordinator = RestoreCoordinator::new(
            boot.restore.clone(),
            layout.clone(),
            session.clone(),
            boot.labels.clone(),
        );
        coordinator.attach(RestorePorts {
            windows: Arc::new(NoWindows),
            events: Arc::new(NoEvents),
            subs: Arc::new(NoSubscriptions),
            // 쓰기를 안 연 설정 — 적재는 없는 폴더를 만들지 않는다.
            themes: ThemeControl::new(
                Arc::new(EffectiveThemes::new(
                    Arc::new(crate::settings::SettingsService::load_from_dir(
                        &state_dir.join("no-settings"),
                    )),
                    layout.clone(),
                )),
                Arc::new(NoThemeWindows),
            ),
        });
        boot.run_steps(&run_dir, &state_dir);
        session.start_saver();

        let reply = coordinator.answer(true).expect("수락");

        assert!(reply.durable, "답을 실은 쓰기가 마감 안에 발행됐다");
        assert_eq!(reply.restored_windows, 1);
        assert!(
            layout.0.lock().unwrap().views.contains_key(&saved_tab),
            "사본의 화면"
        );
        assert!(!state_dir.join(CRASH_COPY_FILE).exists(), "사본을 지운다");
        let written = read_state(&state_dir);
        assert_eq!(
            written.resolved_crash_copy,
            Some(codec::crash_copy_hash(&raw))
        );
        assert_eq!(
            written.windows,
            to_persisted(&layout.0.lock().unwrap()),
            "답을 붙인 쓰기가 수락한 화면을 담는다"
        );
        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }
}
