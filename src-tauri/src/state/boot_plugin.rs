//! Run lifetime of screen state — the boot stage plugin (TRD S21-storage §6-5 ⓪–⑥) · starting the recorder (⑦) · attaching the answer to the disk
//! (§6-7 ⑤) · normal shutdown (§6-6). The field the four share is [`StateSession`], and the recorder's snapshot source is [`LiveSource`].
//!
//! - ★Register the plugin ([`init`]) right after the single-instance plugin★ — plugin setup runs inside `build()` in registration
//!   order, and the single-instance plugin ends the second instance there (§6-5 facts ① · ③). Only after that may the disk
//!   be changed, and the settings window (main) is created later than that, so the model a window first pulls is the decided model.
//! - ★setup does not return `Err`★ — with `Err` the build stops and the app does not come up at all (N7 · D8). Failures are logged and it continues.
//! - ★The restore service state is decided in one place, ⑥ (I5)★ — no window exists yet, so the first `restore_status` pull of every window sees the decided
//!   value. If decided in the user setup, one has to keep by hand that it is earlier than the place that creates windows (⑧ —
//!   `state::placement::restore_windows`).
//! - ★The shell run lock is released explicitly by [`StateSession::shutdown`]★ — when the event loop ends it immediately calls
//!   `process::exit` (tao `platform_impl/windows/event_loop.rs` `run`). Neither managed state nor this field is dropped, so
//!   if we relied on that, the lock would be released only when the OS withdraws the handle and a waiting new instance would be blocked that much longer (`Drop` in `lock.rs`).

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::file::Claim;
use engram_dashboard_base::time::SystemClock;
use engram_dashboard_base::{logging, sync};
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
use super::saver::{self, CloseFlag, RequestOutcome, SaveOutcome, SaverClock, SaverHandle};
use super::schema::WindowEntry;
use crate::discovery::DataLayout;
use crate::layout::{LabelSource, LayoutState, ViewManager, MAIN_WINDOW_LABEL};

const PLUGIN_NAME: &str = "engram-state-boot";

/// What the boot stage fills — passes the same instance that is managed in the builder (ADR-0102 — it exists before the first invoke).
pub struct Boot {
    pub layout: LayoutState,
    /// Labels of restored popouts — it must be the same issuer as the runtime popouts so labels do not overlap (§6-3).
    pub labels: Arc<dyn LabelSource>,
    pub session: Arc<StateSession>,
    /// ⑥ decides the state. The notification is inserted by plugin setup before the boot stage — it announces from the decision of ⑥.
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

/// When the restore state changes, emits [`EVT_RESTORE_CHANGED`] to the main window. ⑥ has no window, so there is no receiver — the window sees the decided value
/// with its first `restore_status` pull (I5).
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
        // ⓪ Writes to a **different file** from the daemon (`app-*.log`) — if two processes share one file, lines get mixed. The folder is the same data folder as the daemon,
        //   so when chasing a startup failure the two logs gather in one place.
        let log_file = logging::init_logging_with_file(&paths.logs_dir(), logging::LogKind::App);
        // If the primary folder cannot be used, the log path can fall under `%TEMP%`, so apart from the return value there is no means to know where it is writing
        //   (same reason as "decide the data folder" in the daemon's `run()`).
        tracing::info!(
            data_dir = %paths.root().display(),
            log_file = ?log_file,
            "앱 로그 파일 결정"
        );
        self.run_steps(&paths.shell_run_dir(), &paths.shell_state_dir());
    }

    /// Boot stage ①–⑥. It does not start the recorder — it leaves the material to start it in [`StateSession`] and ⑦ takes it out.
    fn run_steps(&self, run_dir: &Path, state_dir: &Path) {
        self.session.hold_lock(lock::acquire(run_dir));

        let files = FsBootFiles::in_dir(state_dir);
        let mut plan = boot::prepare(&files);

        let model = match &plan.model {
            BootModel::Default => ViewManager::new(),
            BootModel::Restore(file) => {
                let restored =
                    ViewManager::from_persisted(file.windows.clone(), self.labels.as_ref());
                restored.warnings.iter().for_each(report_restore_warning);
                restored.layout
            }
        };
        let marker = boot::write_run_marker(
            &files,
            &mut plan,
            to_persisted(&model),
            SystemClock.wall_ms(),
        );
        let claim = boot::saver_claim(&plan, marker);

        let revision = {
            // Nobody takes this lock before the boot stage, so it cannot be poisoned — even if it were, the model is swapped as a whole.
            let mut layout = sync::lock(&self.layout.0);
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
        // Guard contract (§6-5 ③) — this run saves ≡ not a guard. If it is a guard, there is no recorder so the answer is not attached to the disk —
        //   the source of guard ⅱ is the in-memory text that the decision carried (L2).
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
            claim,
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

// TRD S21-storage §6-5 — read from the plan after the decision (③) · action (④) · run marker (⑤ — carries a guard if the re-decision just before the write
//   rejects) are finished.
fn state_file_status(plan: &BootPlan) -> StateFileStatus {
    match plan.guard {
        Some(Guard::StateUnreadable(_)) => return StateFileStatus::Unreadable,
        // ADR-0291 R2
        Some(Guard::StateNewer { .. }) => return StateFileStatus::Newer,
        Some(Guard::CrashCopyNotWritten(_)) | None => {}
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

/// The field shared by the boot stage · user setup · `RunEvent::Exit`. The lock is a leaf — while holding it, it takes or waits for no other lock
/// (the wait of `settled` stands after releasing the field lock).
#[derive(Default)]
pub struct StateSession {
    cell: Mutex<SessionCell>,
    /// Wakes when starting (`starting`) ends (loaded · could not start · shutdown) — the answer waits for that end
    /// ([`StateSession::resolve_crash_copy`]).
    settled: Condvar,
}

#[derive(Default)]
struct SessionCell {
    lock: Option<StateLock>,
    /// Set by boot stage ⑥ and taken out by ⑦ ([`StateSession::start_saver`]). `None` = guard (N3 — this run does not save) ·
    /// already taken out.
    saver_start: Option<PendingSaver>,
    /// The recorder's closed flag between taking out the material and loading it into the field — in between, [`StateSession::shutdown`] sets this before releasing the shell run
    /// lock. If not set, a recorder not yet loaded would write after the lock is released and
    /// overwrite the file of a new instance that passed the gate in the meantime.
    starting: Option<CloseFlag>,
    saver: Option<SaverHandle>,
    /// [`StateSession::shutdown`] has already run.
    closed: bool,
}

struct PendingSaver {
    source: LiveSource,
    files: saver::Fs,
    /// Change number of the model that the run marker (⑤) carried.
    revision: StateRevision,
    /// The hash of the answered copy that the run marker (⑤) carried — the recorder carries it on and deletes again.
    carry_resolved: Option<String>,
    /// Has this run already claimed `state.json` (`boot::saver_claim`) — the axis of the recorder's re-decision just before saving.
    claim: Claim,
}

// The answer of [`StateSession::published_saver`].
enum Published {
    Ready(SaverHandle),
    /// Guard · could not start · already ended.
    Absent,
    /// The deadline passed while another thread was still starting.
    StillStarting,
}

/// The answer of [`StateSession::resolve_crash_copy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveResult {
    /// The `state.json` carrying that hash was published within the deadline — it does not carry the success or failure of deleting the copy.
    Durable,
    /// Not attached to the disk — no recorder (guard · could not start · already ended · another starting thread could not load it within the
    /// deadline) · write failure · closed · deadline exceeded.
    NotDurable,
}

impl StateSession {
    fn cell(&self) -> MutexGuard<'_, SessionCell> {
        // The field is only four `Option`s and one flag, so there is no half-changed state — a shutdown path that cannot release the lock is worse.
        sync::lock(&self.cell)
    }

    fn hold_lock(&self, lock: Option<StateLock>) {
        self.cell().lock = lock;
    }

    /// Boot stage ⑦ — if it was a guard or already started (including the case where an answer that came before ⑦ started it first —
    /// [`Self::resolve_crash_copy`]), it does nothing.
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
            pending.claim,
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

    // Takes out the material to start and sets that recorder's closed flag in the field as 「띄우는 중」 (starting) — the one who took it out must always
    // finish with [`Self::publish_saver`] or [`Self::end_start`] (if not finished, the answer waits until the deadline).
    fn take_pending(&self) -> Option<(PendingSaver, CloseFlag)> {
        let mut cell = self.cell();
        let pending = cell.saver_start.take()?;
        let closed = CloseFlag::default();
        cell.starting = Some(closed.clone());
        Some((pending, closed))
    }

    // Withdraws 「띄우는 중」 (starting) and wakes the waiting answers — `then` runs inside the field lock (only assign fields).
    fn end_start<T>(&self, then: impl FnOnce(&mut SessionCell) -> T) -> T {
        let out = {
            let mut cell = self.cell();
            cell.starting = None;
            then(&mut cell)
        };
        self.settled.notify_all();
        out
    }

    // Loads the started recorder into the field — if [`Self::shutdown`] ran during starting (outside the lock), it does not load (shutdown has already closed that
    // recorder — `starting`).
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

    /// Passes the answered copy's hash to the recorder (`Resolve`) and waits until the deadline ([`saver::REPLY_DEADLINE`]) (TRD §6-7 ⑤).
    ///
    /// ★If the recorder does not exist yet, it first calls ⑦ ([`Self::start_saver`])★ — so that even if the answer comes before ⑦, it does not become
    /// `NotDurable` with a recorder about to come up (it does not happen in today's boot order — windows are created in ⑧ after ⑦). After a guard · already started ·
    /// shutdown, that call does nothing.
    ///
    /// ★If another thread (⑦ · another answer) is starting, it waits until loaded or failed★ — that wait and the `Resolve` answer wait
    /// share one deadline. If it did not wait, it would look at that moment as if there were no recorder and the answer would not attach to the disk.
    ///
    /// ★It waits holding no lock — the caller also does not call it holding the layout lock★: the recorder takes
    /// that lock to take a snapshot (`saver::SnapshotSource`). If held, the answer always exceeds the deadline.
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

    // If it is starting, waits for its end (loaded · could not start · shutdown) until `until` and returns the recorder that was loaded. The field lock is released
    // while waiting — the one starting must take that lock to load. The caller leaves the log (one line per branch).
    fn published_saver(&self, until: Instant) -> Published {
        let mut cell = self.cell();
        while cell.starting.is_some() && !cell.closed {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Published::StillStarting;
            }
            cell = sync::wait_timeout(&self.settled, cell, left).0;
        }
        match cell.saver.clone() {
            Some(saver) => Published::Ready(saver),
            None => Published::Absent,
        }
    }

    /// `RunEvent::Exit` — waits for the normal shutdown write (`Final`) until the deadline ([`saver::REPLY_DEADLINE`]) and then releases the shell run
    /// lock (§6-6). If there is no recorder (guard · could not start), it only releases the lock (N7). A second call does nothing.
    ///
    /// ★Called on the main thread — it waits holding no lock★: the recorder takes the layout lock to take a snapshot
    /// (`saver::SnapshotSource`).
    pub fn shutdown(&self) {
        let (saver, lock) = {
            let mut cell = self.cell();
            if std::mem::replace(&mut cell.closed, true) {
                return;
            }
            cell.saver_start = None;
            // Before releasing the lock — a recorder that is starting is not loaded yet, so the `Final` below does not reach it (field head
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
            // What exceeded the deadline was already warned by `finish`.
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

/// The recorder's snapshot source — the layout model. Window getters are not called (position · size read the values written in the model).
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
        // Reading the number is harmless even on a half-changed model — not using a poisoned model is done by `snapshot`.
        let layout = sync::lock(&self.layout.0);
        StateRevision::of(&layout)
    }

    fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
        // A poisoned model is not used — a panicked change may remain half-done, and if used, the next boot restores that shape.
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
    use crate::commands::popout::{popup_label, PopupCounter};
    use crate::layout::{LayoutEvents, TabName, ViewSnapshot, WindowBounds, WindowTabsPayload};
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

    /// A screen with one tab name changed — the `state.json` text left by the previous run and that tab.
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

    // ── snapshot source ──

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

    // ── boot stage ①–⑥ · ⑦ · shutdown ──

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
            assert_eq!(mgr.views[&saved_tab].name, TabName::Named("지난 탭".into()));
            assert_eq!(mgr.windows.len(), 2, "main + 복원한 팝아웃");
            assert!(
                mgr.windows.contains_key(&popup_label(1)),
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
        // A folder is neither 「없음」 (none) nor 「못 쓸 파일」 (unusable file) but a read IO failure (I3) — guard.
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

    /// ADR-0291 R2: a `state.json` written by a newer version is a guard — announced as `newer` · without a recorder · file bytes as they are.
    #[test]
    fn a_newer_state_file_is_reported_as_newer_and_left_byte_for_byte() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        let newer = r#"{"version":2,"saved_at_ms":1,"clean_exit":true,"windows":[]}"#;
        std::fs::write(state_dir.join(STATE_FILE), newer).unwrap();
        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(boot.restore.status().state_file, StateFileStatus::Newer);
        assert!(!boot.restore.status().saves, "가드 — 저장하지 않는다");
        assert!(session.cell().saver_start.is_none(), "기록기 재료가 없다");
        session.shutdown();
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            newer
        );
        assert!(
            !state_dir.join("state.json.corrupt").exists(),
            "떠 두지 않는다"
        );
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
        // A non-empty folder is in the place to copy aside — the rename that swaps it for a file fails.
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
            claim: Claim::NotYet,
        };
        let unreadable = || Some(Guard::StateUnreadable("잠김".into()));
        let not_written = || Some(Guard::CrashCopyNotWritten("새 판의 사본".into()));
        for (guard, aside, expected) in [
            (None, None, StateFileStatus::Ok),
            (unreadable(), None, StateFileStatus::Unreadable),
            (
                Some(Guard::StateNewer { found: 2 }),
                None,
                StateFileStatus::Newer,
            ),
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

    // ── crash copy · restore service (⑥) · attaching the answer to the disk ──

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
        // A folder is neither 「없음」 (none) nor 「못 쓸 파일」 (unusable file) but a read IO failure (I3) — it is not known whether it may be overwritten.
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

    // For guard ⅱ, once `state_file` has answered `ok`, the only field that carries that fact is `saves` (guard ⅱ notice — ADR-0276).
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
        // A folder is neither 「없음」 (none) nor 「못 쓸 파일」 (unusable file) but a read IO failure (I3) — guard ⅰ.
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

        // Pries open the gap of `start_saver` by hand — shutdown runs while the material is taken out and started outside the lock.
        let handle = spawn_pending(&session);
        let witness = handle.clone();
        session.shutdown();
        assert!(lock_is_free(&run_dir), "종료가 잠금을 놓았다");
        // Look before loading — now that the lock is released it must already be closed (shutdown set it before the lock).
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

    /// The front half of `start_saver` — takes out the material, sets 「띄우는 중」 (starting) and starts (does not load).
    fn spawn_pending(session: &StateSession) -> SaverHandle {
        let (pending, closed) = session.take_pending().expect("기록기 재료");
        saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            pending.carry_resolved,
            pending.claim,
            closed,
        )
        .expect("기록기 스레드")
    }

    /// Runs a boot with a crash copy up to ⑥ — its session · lock folder · state folder · copy hash.
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
            pending.claim,
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

    // ── restore coordinator — from the boot stage until the answer attaches to the disk ──

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
            // Settings with writes not opened — the load does not create a missing folder.
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
