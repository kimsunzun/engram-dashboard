//! Run lifetime of screen state — the boot stage plugin (TRD S21-storage §6-5 ⓪–⑥) · starting the recorder (⑦) · attaching the answer to disk (§6-7 ⑤) · normal shutdown (§6-6). The field the four share is [`StateSession`]; the recorder's snapshot source is [`LiveSource`].
//! - Register the plugin ([`init`]) right after the single-instance plugin: plugin setup runs inside `build()` in registration order and the single-instance plugin ends the second instance there (§6-5 facts ① · ③); only after that may the disk be changed, and the settings window (main) is created later, so the model a window first pulls is the decided one.
//! - setup does not return `Err` — with `Err` the build stops and the app does not come up (N7 · D8); failures are logged and it continues.
//! - The restore service state is decided in one place, ⑥ (I5): no window exists yet, so every window's first `restore_status` pull sees the decided value. Decided in the user setup, one would have to keep by hand that it precedes the window-creating step (⑧ — `state::placement::restore_windows`).
//! - The shell run lock is released explicitly by [`StateSession::shutdown`]: when the event loop ends it calls `process::exit` at once (tao `platform_impl/windows/event_loop.rs` `run`), dropping neither managed state nor this field — relying on that would release the lock only when the OS withdraws the handle and a waiting new instance would block that much longer (`Drop` in `lock.rs`).

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
use crate::settings::SettingsService;
use crate::ui_text::Lang;

const PLUGIN_NAME: &str = "engram-state-boot";

/// What the boot stage fills — the same instances managed in the builder, so they exist before the first invoke (ADR-0102).
pub struct Boot {
    pub layout: LayoutState,
    /// Must be the same issuer as runtime popouts so labels do not overlap (§6-3).
    pub labels: Arc<dyn LabelSource>,
    pub session: Arc<StateSession>,
    pub restore: Arc<RestoreService>,
    /// The loaded settings — the boot model takes the effective `ui.language` before anything reads a name from it.
    pub settings: Arc<SettingsService>,
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

/// Emits [`EVT_RESTORE_CHANGED`] to the main window when the restore state changes. ⑥ has no window, so no receiver — the window sees the decided value with its first `restore_status` pull (I5).
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
                "could not notify the main window of the restore status change"
            );
        }
    }
}

impl Boot {
    fn run(self) {
        let paths = DataLayout::resolve();
        // ⓪ Writes to a different file from the daemon (`app-*.log`) — two processes sharing one file mix lines. The folder is the same data folder as the daemon, so a startup failure's two logs gather in one place.
        let log_file = logging::init_logging_with_file(&paths.logs_dir(), logging::LogKind::App);
        // If the primary folder is unusable the log path can fall under `%TEMP%`, so apart from the return value nothing tells where it writes (same reason as "decide the data folder" in the daemon's `run()`).
        tracing::info!(
            data_dir = %paths.root().display(),
            log_file = ?log_file,
            "app log file decided"
        );
        self.run_steps(&paths.shell_run_dir(), &paths.shell_state_dir());
    }

    fn run_steps(&self, run_dir: &Path, state_dir: &Path) {
        self.session.hold_lock(lock::acquire(run_dir));

        let files = FsBootFiles::in_dir(state_dir);
        let mut plan = boot::prepare(&files);

        let mut model = match &plan.model {
            BootModel::Default => ViewManager::new(),
            BootModel::Restore(file) => {
                let restored =
                    ViewManager::from_persisted(file.windows.clone(), self.labels.as_ref());
                restored.warnings.iter().for_each(report_restore_warning);
                restored.layout
            }
        };
        // Before the run marker (it saves the names) and the swap — so a restart in Korean shows Korean names and titles from the first frame. Nobody has seen this model, so the windows it reports need no notification.
        // ADR-0373
        model.set_language(Lang::from_settings(&self.settings));
        let marker = boot::write_run_marker(
            &files,
            &mut plan,
            to_persisted(&model),
            SystemClock.wall_ms(),
        );
        let claim = boot::saver_claim(&plan, marker);

        let revision = {
            // Nobody takes this lock before the boot stage, so it cannot be poisoned; even if it were, the model is swapped whole.
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
        // Guard contract (§6-5 ③): this run saves ≡ not a guard. A guard has no recorder, so the answer is not attached to disk — guard ⅱ's source is the in-memory text the decision carried (L2).
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
            "boot stage — model · restore status decided"
        );
        self.session.cell().saver_start = saver;
    }
}

// TRD S21-storage §6-5 — read from the plan after the decision (③) · action (④) · run marker (⑤ — carries a guard if the re-decision just before the write rejects) are finished.
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
            tracing::error!(module = "state", %warning, "screen state restore")
        }
        _ => tracing::warn!(module = "state", %warning, "screen state restore"),
    }
}

/// The field shared by the boot stage · user setup · `RunEvent::Exit`. The lock is a leaf — while holding it, take or wait for no other lock (`settled`'s wait stands after releasing it).
#[derive(Default)]
pub struct StateSession {
    cell: Mutex<SessionCell>,
    /// Wakes when starting (`starting`) ends (loaded · could not start · shutdown) — the answer waits for that end ([`StateSession::resolve_crash_copy`]).
    settled: Condvar,
}

#[derive(Default)]
struct SessionCell {
    lock: Option<StateLock>,
    /// Set by boot stage ⑥ and taken out by ⑦ ([`StateSession::start_saver`]). `None` = guard (N3 — this run does not save) · already taken out.
    saver_start: Option<PendingSaver>,
    /// The recorder's closed flag between taking out the material and loading it into the field — [`StateSession::shutdown`] sets it before releasing the shell run lock; unset, a not-yet-loaded recorder would write after the lock is released and overwrite the file of a new instance that passed the gate meanwhile.
    starting: Option<CloseFlag>,
    saver: Option<SaverHandle>,
    closed: bool,
}

struct PendingSaver {
    source: LiveSource,
    files: saver::Fs,
    revision: StateRevision,
    carry_resolved: Option<String>,
    claim: Claim,
}

enum Published {
    Ready(SaverHandle),
    Absent,
    StillStarting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveResult {
    Durable,
    /// Not attached to disk — no recorder (guard · could not start · already ended · another starting thread could not load it within the deadline) · write failure · closed · deadline exceeded.
    NotDurable,
}

impl StateSession {
    fn cell(&self) -> MutexGuard<'_, SessionCell> {
        // Only four `Option`s and one flag, so no half-changed state; a shutdown path that cannot release the lock would be worse.
        sync::lock(&self.cell)
    }

    fn hold_lock(&self, lock: Option<StateLock>) {
        self.cell().lock = lock;
    }

    /// Boot stage ⑦ — does nothing if it was a guard or already started (including an answer that came before ⑦ and started it first — [`Self::resolve_crash_copy`]).
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
                    "could not start the writer thread — this run saves no more screen state"
                );
            }
        }
    }

    // Takes out the material to start and sets that recorder's closed flag in the field as 「starting」 — the taker must always finish with [`Self::publish_saver`] or [`Self::end_start`] (else the answer waits until the deadline).
    fn take_pending(&self) -> Option<(PendingSaver, CloseFlag)> {
        let mut cell = self.cell();
        let pending = cell.saver_start.take()?;
        let closed = CloseFlag::default();
        cell.starting = Some(closed.clone());
        Some((pending, closed))
    }

    fn end_start<T>(&self, then: impl FnOnce(&mut SessionCell) -> T) -> T {
        let out = {
            let mut cell = self.cell();
            cell.starting = None;
            then(&mut cell)
        };
        self.settled.notify_all();
        out
    }

    // Loads the started recorder into the field — if [`Self::shutdown`] ran during starting (outside the lock) it is not loaded (shutdown already closed that recorder via `starting`).
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
                "the exit ran while the writer was starting, so that writer is closed — nothing is written"
            );
        }
    }

    /// Passes the answered copy's hash to the recorder (`Resolve`) and waits until the deadline ([`saver::REPLY_DEADLINE`]) (TRD §6-7 ⑤).
    /// If the recorder does not exist yet it first calls ⑦ ([`Self::start_saver`]), so an answer before ⑦ does not become `NotDurable` with a recorder about to come up (not today's boot order — windows are created in ⑧ after ⑦); after a guard · already started · shutdown that call does nothing. If another thread (⑦ · another answer) is starting it waits until loaded or failed, sharing one deadline with the `Resolve` wait — else it would see no recorder and the answer would not attach to disk.
    /// Waits holding no lock, and the caller must not hold the layout lock: the recorder takes it to snapshot (`saver::SnapshotSource`), so held, the answer always exceeds the deadline.
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
                    "there is no writer, so the answer is not attached to disk"
                );
                return ResolveResult::NotDurable;
            }
            Published::StillStarting => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "the writer did not start within the deadline, so the answer is not attached to disk"
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
                    "could not attach the answer to disk within the deadline"
                );
                ResolveResult::NotDurable
            }
        }
    }

    // If starting, waits for its end (loaded · could not start · shutdown) until `until` and returns the recorder that was loaded. The field lock is released while waiting — the starter must take it to load. The caller leaves the log (one line per branch).
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

    /// `RunEvent::Exit` — waits for the normal shutdown write (`Final`) until the deadline ([`saver::REPLY_DEADLINE`]) and then releases the shell run lock (§6-6); with no recorder (guard · could not start) it only releases the lock (N7). A second call does nothing. Called on the main thread, waiting holding no lock: the recorder takes the layout lock to snapshot (`saver::SnapshotSource`).
    pub fn shutdown(&self) {
        let (saver, lock) = {
            let mut cell = self.cell();
            if std::mem::replace(&mut cell.closed, true) {
                return;
            }
            cell.saver_start = None;
            // Before releasing the lock — a recorder that is starting is not loaded yet, so the `Final` below does not reach it (field head `starting`).
            if let Some(starting) = cell.starting.take() {
                starting.close();
            }
            (cell.saver.take(), cell.lock.take())
        };
        self.settled.notify_all();
        match saver.map(|saver| saver.finish(saver::REPLY_DEADLINE)) {
            Some(RequestOutcome::Done(SaveOutcome::Written)) => {
                tracing::info!(module = "state", "recorded the normal exit")
            }
            Some(RequestOutcome::TimedOut) => {}
            Some(outcome) => tracing::warn!(
                module = "state",
                ?outcome,
                "could not record the normal exit — the next boot reads it as an abnormal exit"
            ),
            None => tracing::info!(module = "state", "no writer, so it ends without the normal exit write"),
        }
        drop(lock);
    }
}

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
        let layout = sync::lock(&self.layout.0);
        StateRevision::of(&layout)
    }

    fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
        // A poisoned model is not used — a panicked change may remain half-done, and if used, the next boot restores that shape.
        let layout = self
            .layout
            .0
            .lock()
            .map_err(|_| "the layout lock is poisoned".to_string())?;
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
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp folder");
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
        codec::decode(&text).expect("state file").0
    }

    fn lock_is_free(run_dir: &Path) -> bool {
        let file = OpenOptions::new()
            .write(true)
            .open(run_dir.join(LOCK_FILE))
            .expect("lock file");
        let free = file.try_lock().is_ok();
        if free {
            file.unlock().expect("release the held lock");
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
            settings: Arc::new(SettingsService::load_from_dir(
                &temp_dir("settings").join("absent"),
            )),
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

    fn previous_run(clean_exit: bool, resolved: Option<String>) -> (String, uuid::Uuid) {
        let mut previous = ViewManager::new();
        let view = previous.windows[MAIN_WINDOW_LABEL].active;
        previous.rename_tab(view, "Last tab".into()).unwrap();
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
        assert_eq!(source.revision(), start, "the same when nothing changed");

        let view = main_view(&layout);
        layout
            .0
            .lock()
            .unwrap()
            .rename_tab(view, "New name".into())
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
            .rename_tab(view, "Tab to save".into())
            .unwrap();
        layout
            .0
            .lock()
            .unwrap()
            .set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
            .unwrap();

        let windows = source.snapshot().expect("snapshot");
        let expected = to_persisted(&layout.0.lock().unwrap());
        assert_eq!(windows, expected);
    }

    #[test]
    fn a_poisoned_layout_lock_fails_the_snapshot_without_panicking() {
        let (source, layout) = source();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = layout.0.lock().unwrap();
            panic!("poison");
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
            previous.rename_tab(view, "Last tab".into()).unwrap();
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
            "⑥ decides once (I5)"
        );
        {
            let mgr = layout.0.lock().unwrap();
            assert_eq!(mgr.views[&saved_tab].name, TabName::Named("Last tab".into()));
            assert_eq!(mgr.windows.len(), 2, "main + the restored popout");
            assert!(
                mgr.windows.contains_key(&popup_label(1)),
                "the popout receives a new label from this run's issuer"
            );
            assert_eq!(
                mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme,
                Some(UiTheme::Dark)
            );
        }
        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit, "run marker");
        assert_eq!(marker.windows.len(), 2, "main · popout");
        assert!(!lock_is_free(&run_dir), "the lock is held until exit");

        session.start_saver();
        session.shutdown();
        assert!(read_state(&state_dir).clean_exit, "normal exit write");
        assert!(lock_is_free(&run_dir), "the exit releases the lock");
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
            "a missing file is ok"
        );
        assert!(boot.restore.status().saves, "not a guard — it saves");
        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit);
        assert_eq!(
            marker.windows,
            to_persisted(&layout.0.lock().unwrap()),
            "what the marker holds is the memory model"
        );
    }

    #[test]
    fn a_korean_setting_names_the_boot_model_in_korean_before_any_write() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state").join("absent");
        let config = temp_dir("settings-ko");
        std::fs::write(config.join("settings.json"), r#"{"ui.language":"ko"}"#).unwrap();
        let layout = LayoutState::new();
        let (mut boot, _session) = boot(&layout);
        boot.settings = Arc::new(SettingsService::load_from_dir(&config));
        boot.run_steps(&run_dir, &state_dir);

        let mgr = layout.0.lock().unwrap();
        assert_eq!(mgr.language(), Lang::Ko);
        let tabs = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(tabs.tabs[0].name, "빈 칸"); // hangul-ok: the Korean UI table
        assert_eq!(
            crate::layout::apply::WindowTabsPayload::from(tabs).title,
            "빈 칸 - Engram:1" // hangul-ok: the Korean UI table
        );
        assert_eq!(
            read_state(&state_dir).windows,
            to_persisted(&mgr),
            "the run marker saved the Korean names"
        );
    }

    #[test]
    fn the_default_setting_boots_in_english() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state").join("absent");
        let layout = LayoutState::new();
        let (boot, _session) = boot(&layout);
        boot.run_steps(&run_dir, &state_dir);

        let mgr = layout.0.lock().unwrap();
        assert_eq!(mgr.language(), Lang::En);
        assert_eq!(
            mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap().tabs[0].name,
            "Empty slot"
        );
    }

    #[test]
    fn an_unreadable_state_file_guards_the_run_and_never_starts_a_saver() {
        let run_dir = temp_dir("run");
        let state_dir = temp_dir("state");
        // A folder is neither 「none」 nor 「unusable file」 but a read IO failure (I3) — guard.
        std::fs::create_dir(state_dir.join(STATE_FILE)).unwrap();
        let layout = LayoutState::new();
        let (boot, session) = boot(&layout);
        boot.run_steps(&run_dir, &state_dir);

        assert_eq!(boot.restore.status().crash_copy, CrashCopyStatus::None);
        assert_eq!(
            boot.restore.status().state_file,
            StateFileStatus::Unreadable,
            "guard ⅰ — ⑥ decides even without a copy"
        );
        assert!(!boot.restore.status().saves, "guard ⅰ — does not save");
        assert!(session.cell().saver_start.is_none(), "no writer material");
        assert!(
            state_dir.join(STATE_FILE).is_dir(),
            "does not write the run marker"
        );
        session.start_saver();
        assert!(
            session.cell().saver.is_none(),
            "⑦ starts nothing"
        );

        session.shutdown();
        assert!(lock_is_free(&run_dir), "even with a guard the exit releases the lock");
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
        assert!(!boot.restore.status().saves, "guard — does not save");
        assert!(session.cell().saver_start.is_none(), "no writer material");
        session.shutdown();
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            newer
        );
        assert!(
            !state_dir.join("state.json.corrupt").exists(),
            "not copied aside"
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
            "an unusable file is not a guard — it saves"
        );
        assert_eq!(
            notices.seen(),
            [CrashCopyStatus::None],
            "⑥ decides even without a copy"
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
            "a copy-aside failure is not a guard either (D8)"
        );
        assert!(
            !read_state(&state_dir).clean_exit,
            "the run marker replaced the original (D8)"
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
        let unreadable = || Some(Guard::StateUnreadable("locked".into()));
        let not_written = || Some(Guard::CrashCopyNotWritten("a newer version's copy".into()));
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
            "copy = the previous run's state.json raw text"
        );
        assert_eq!(
            notices.seen(),
            [CrashCopyStatus::Awaiting],
            "⑥ decides once (I5)"
        );
        assert_eq!(boot.restore.status().windows, Some(1), "main only");
        let status = boot.restore.status();
        assert!(status.saves, "not a guard — this run saves");
        assert_eq!(status.durable, Some(true), "while asking it equals saves");
        assert!(
            !layout.0.lock().unwrap().views.contains_key(&saved_tab),
            "the default screen while asking"
        );
        assert!(!read_state(&state_dir).clean_exit, "run marker");
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
            "after attaching the answer to disk the copy is removed"
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
            "a normal exit does not remove the copy (D6)"
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
        // A folder is neither 「none」 nor 「unusable file」 but a read IO failure (I3) — it is not known whether it may be overwritten.
        std::fs::create_dir(state_dir.join(CRASH_COPY_FILE)).unwrap();

        let (boot, session) = boot(&LayoutState::new());
        boot.run_steps(&run_dir, &state_dir);

        let status = boot.restore.status();
        assert!(!status.saves, "guard ⅱ — this run does not save");
        assert_eq!(status.durable, Some(false), "while asking it equals saves");
        assert_eq!(
            status.state_file,
            StateFileStatus::Ok,
            "guard ⅱ is not put into state_file"
        );
        {
            let ticket = boot.restore.begin_answer().expect("asks even under guard ⅱ");
            assert_eq!(
                ticket.copy().text,
                raw,
                "restore source = the state.json raw text in memory (L2)"
            );
        }
        assert!(
            session.cell().saver_start.is_none(),
            "guard — no writer material"
        );
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            raw,
            "the state.json bytes on disk as is"
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

        let reply = coordinator.answer(false).expect("decline");

        assert!(!reply.durable, "no writer");
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
        // A folder is neither 「none」 nor 「unusable file」 but a read IO failure (I3) — guard ⅰ.
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
        assert!(!answered.saves, "unchanged even after answering");
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
        assert!(session.cell().saver.is_none(), "before ⑦");

        let hash = codec::crash_copy_hash(&raw);
        assert_eq!(
            session.resolve_crash_copy(hash.clone()),
            ResolveResult::Durable
        );
        assert!(!state_dir.join(CRASH_COPY_FILE).exists());
        assert_eq!(read_state(&state_dir).resolved_crash_copy, Some(hash));
        assert!(
            session.cell().saver_start.is_none(),
            "the answer used ⑦'s material"
        );

        session.start_saver();
        session.shutdown();
        assert!(
            read_state(&state_dir).clean_exit,
            "the writer the answer started records the normal exit"
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
        assert!(lock_is_free(&run_dir), "the exit released the lock");
        // Look before loading — now that the lock is released it must already be closed (shutdown set it before the lock).
        assert_eq!(
            witness.resolve(codec::crash_copy_hash(&raw), saver::REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped),
            "a writer not loaded after the lock was released does not write"
        );
        session.publish_saver(handle);

        assert!(session.cell().saver.is_none(), "not loaded into a closed session");
        assert!(session.cell().starting.is_none());
        assert_eq!(
            session.resolve_crash_copy(codec::crash_copy_hash(&raw)),
            ResolveResult::NotDurable
        );
        assert_eq!(
            std::fs::read_to_string(state_dir.join(STATE_FILE)).unwrap(),
            marker,
            "state.json unchanged after the exit"
        );
        assert!(state_dir.join(CRASH_COPY_FILE).exists(), "the copy unchanged too");
    }

    fn spawn_pending(session: &StateSession) -> SaverHandle {
        let (pending, closed) = session.take_pending().expect("writer material");
        saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            pending.carry_resolved,
            pending.claim,
            closed,
        )
        .expect("writer thread")
    }

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

        let (pending, closed) = session.take_pending().expect("writer material");
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
        .expect("writer thread");

        assert_eq!(
            handle.resolve(hash, saver::REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped),
            "a writer started after the lock was released is closed from the start"
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
            "while starting, it does not end saying there is no writer but waits"
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
        assert!(waited < saver::REPLY_DEADLINE, "does not stand until the deadline");
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

        let reply = coordinator.answer(true).expect("accept");

        assert!(reply.durable, "the write carrying the answer was issued within the deadline");
        assert_eq!(reply.restored_windows, 1);
        assert!(
            layout.0.lock().unwrap().views.contains_key(&saved_tab),
            "the copy's screen"
        );
        assert!(!state_dir.join(CRASH_COPY_FILE).exists(), "the copy is removed");
        let written = read_state(&state_dir);
        assert_eq!(
            written.resolved_crash_copy,
            Some(codec::crash_copy_hash(&raw))
        );
        assert_eq!(
            written.windows,
            to_persisted(&layout.0.lock().unwrap()),
            "the write with the answer attached holds the accepted screen"
        );
        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }
}
