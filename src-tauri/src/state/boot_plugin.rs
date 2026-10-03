//! 화면 상태의 실행 수명 — 부팅 단계 플러그인(TRD S21-storage §6-5 ⓪–⑥) · 기록기 시작(⑦) · 정상 종료(§6-6). 셋이
//! 나눠 쥐는 칸이 [`StateSession`] 이고, 기록기의 스냅숏 원천이 [`LiveSource`] 다.
//!
//! - ★플러그인([`init`])은 단일 인스턴스 플러그인 바로 뒤에 등록한다★ — 플러그인 setup 은 `build()` 안에서 등록
//!   순서대로 돌고 단일 인스턴스 플러그인이 거기서 둘째 인스턴스를 끝낸다(§6-5 사실 ① · ③). 그 뒤라야 디스크를
//!   바꿔도 되고, 설정 창(main · agent-tree)은 그보다 늦게 만들어지므로 창이 처음 당기는 모델이 판정한 모델이다.
//! - ★setup 은 `Err` 를 돌려주지 않는다★ — `Err` 면 빌드가 멈춰 앱이 아예 안 뜬다(N7 · D8). 실패는 log 하고 계속한다.
//! - ★셸 실행 잠금은 [`StateSession::shutdown`] 이 명시적으로 놓는다★ — 이벤트 루프는 끝나면 곧장
//!   `process::exit` 한다(tao `platform_impl/windows/event_loop.rs` `run`). managed state 도 이 칸도 drop 되지 않아,
//!   기대면 잠금은 OS 가 핸들을 거둘 때에야 풀리고 기다리는 새 인스턴스가 그만큼 더 막힌다(`lock.rs` 의 `Drop`).

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_dashboard_base::logging;
use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::Runtime;

use super::boot::{self, BootModel, FsBootFiles, CRASH_COPY_FILE, STATE_FILE};
use super::convert::{to_persisted, RestoreWarning, StateRevision};
use super::lock::{self, StateLock};
use super::saver::{self, Clock, RequestOutcome, SaveOutcome, SaverHandle, SystemClock};
use super::schema::WindowEntry;
use super::tree_attrs::TreeAttrs;
use crate::discovery::DataLayout;
use crate::layout::{LabelSource, LayoutState, ViewManager, WindowAttrs};

const PLUGIN_NAME: &str = "engram-state-boot";

/// 부팅 단계가 채우는 것 — 빌더에서 manage 하는 것과 같은 인스턴스를 넘긴다(ADR-0102 — 첫 invoke 전에 있다).
pub struct Boot {
    pub layout: LayoutState,
    pub tree: Arc<TreeAttrs>,
    /// 복원하는 팝아웃의 label — 런타임 팝아웃과 같은 발급기여야 label 이 겹치지 않는다(§6-3).
    pub labels: Arc<dyn LabelSource>,
    pub session: Arc<StateSession>,
}

pub fn init<R: Runtime>(boot: Boot) -> TauriPlugin<R> {
    PluginBuilder::new(PLUGIN_NAME)
        .setup(move |_app, _api| {
            boot.run();
            Ok(())
        })
        .build()
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
        self.restore(&paths.shell_run_dir(), &paths.shell_state_dir());
    }

    /// 부팅 단계 ①–⑥. 기록기는 띄우지 않는다 — 띄울 재료를 [`StateSession`] 에 맡기고 ⑦ 이 꺼낸다.
    fn restore(&self, run_dir: &Path, state_dir: &Path) {
        self.session.hold_lock(lock::acquire(run_dir));

        let files = FsBootFiles::in_dir(state_dir);
        let plan = boot::prepare(&files);

        let (model, tree) = match &plan.model {
            BootModel::Default => (ViewManager::new(), WindowAttrs::default()),
            BootModel::Restore(file) => {
                let restored =
                    ViewManager::from_persisted(file.windows.clone(), self.labels.as_ref());
                restored.warnings.iter().for_each(report_restore_warning);
                (restored.layout, restored.tree)
            }
        };
        let marker = boot::write_run_marker(
            &files,
            &plan,
            to_persisted(&model, tree),
            SystemClock.wall_ms(),
        );

        // 트리 칸과 레이아웃 락을 겹쳐 잡지 않는다(§6-3).
        self.tree.set(tree);
        let tree_rev = self.tree.rev();
        let revision = {
            // 이 락을 부팅 단계보다 먼저 잡는 쪽이 없어 독이 들 수 없다 — 들었어도 모델을 통째로 갈아끼운다.
            let mut layout = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
            *layout = model;
            StateRevision::of(&layout, tree_rev)
        };

        let saver = plan.guard.is_none().then(|| PendingSaver {
            source: LiveSource::new(self.layout.clone(), self.tree.clone()),
            files: saver::Fs::new(state_dir.join(STATE_FILE), state_dir.join(CRASH_COPY_FILE)),
            revision,
        });
        tracing::info!(
            module = "state",
            restore = matches!(plan.model, BootModel::Restore(_)),
            marker = ?marker,
            saver = saver.is_some(),
            "부팅 단계 — 모델을 채웠다"
        );
        self.session.cell().saver_start = saver;
    }
}

fn report_restore_warning(warning: &RestoreWarning) {
    match warning {
        RestoreWarning::Internal(_) => {
            tracing::error!(module = "state", %warning, "화면 상태 복원")
        }
        _ => tracing::warn!(module = "state", %warning, "화면 상태 복원"),
    }
}

/// 부팅 단계 · 사용자 setup · `RunEvent::Exit` 가 나눠 쥐는 칸. 락은 잎이다 — 쥔 채 다른 락을 잡거나 기다리지 않는다.
#[derive(Default)]
pub struct StateSession {
    cell: Mutex<SessionCell>,
}

#[derive(Default)]
struct SessionCell {
    lock: Option<StateLock>,
    /// 부팅 단계 ⑥ 이 세우고 ⑦([`StateSession::start_saver`])이 꺼낸다. `None` = 가드(N3 — 이 실행은 저장하지
    /// 않는다) · 이미 꺼냈다.
    saver_start: Option<PendingSaver>,
    saver: Option<SaverHandle>,
    /// [`StateSession::shutdown`] 가 이미 돌았다.
    closed: bool,
}

struct PendingSaver {
    source: LiveSource,
    files: saver::Fs,
    /// 실행 표식(⑤)이 담은 모델의 변경 번호.
    revision: StateRevision,
}

impl StateSession {
    fn cell(&self) -> MutexGuard<'_, SessionCell> {
        // 칸이 `Option` 셋과 깃발 하나뿐이라 반쯤 바뀐 상태가 없다 — 종료 경로가 잠금을 놓지 못하는 것이 더 나쁘다.
        self.cell.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hold_lock(&self, lock: Option<StateLock>) {
        self.cell().lock = lock;
    }

    /// 부팅 단계 ⑦ — 가드였거나 이미 띄웠으면 아무것도 하지 않는다.
    pub fn start_saver(&self) {
        let Some(pending) = self.cell().saver_start.take() else {
            return;
        };
        // TODO(P3c1): 부팅이 못 지운 답한 사본의 해시(`carry_resolved`)를 넘긴다.
        match saver::spawn(
            pending.source,
            pending.files,
            SystemClock,
            pending.revision,
            None,
        ) {
            Ok(handle) => self.cell().saver = Some(handle),
            Err(e) => tracing::error!(
                module = "state",
                error = %e,
                "기록기 스레드를 띄우지 못했다 — 이 실행은 화면 상태를 더 저장하지 않는다"
            ),
        }
    }

    /// `RunEvent::Exit` — 정상 종료 쓰기(`Final`)를 마감([`saver::REPLY_DEADLINE`])까지 기다린 뒤 셸 실행 잠금을
    /// 놓는다(§6-6). 기록기가 없으면(가드 · 못 띄움) 잠금만 놓는다(N7). 두 번째 부름은 아무것도 하지 않는다.
    ///
    /// ★메인 스레드에서 불린다 — 아무 락도 쥐지 않고 기다린다★: 기록기는 스냅숏을 뜨려고 레이아웃 · 트리 칸 락을
    /// 잡는다(`saver::SnapshotSource`).
    pub fn shutdown(&self) {
        let (saver, lock) = {
            let mut cell = self.cell();
            if std::mem::replace(&mut cell.closed, true) {
                return;
            }
            cell.saver_start = None;
            (cell.saver.take(), cell.lock.take())
        };
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

/// 기록기의 스냅숏 원천 — 레이아웃 모델과 트리 창 칸. 둘을 겹쳐 잡지 않는다 — 트리 칸을 먼저 읽고 놓은 뒤 레이아웃
/// 락을 잡는다(§6-3). 창 게터는 부르지 않는다(위치 · 크기는 모델에 적힌 값을 읽는다).
pub struct LiveSource {
    layout: LayoutState,
    tree: Arc<TreeAttrs>,
}

impl LiveSource {
    pub fn new(layout: LayoutState, tree: Arc<TreeAttrs>) -> Self {
        Self { layout, tree }
    }
}

impl saver::SnapshotSource for LiveSource {
    type Revision = StateRevision;

    fn revision(&self) -> StateRevision {
        let tree = self.tree.rev();
        // 번호 읽기는 반쯤 바뀐 모델에도 해롭지 않다 — 독 든 모델을 쓰지 않는 것은 `snapshot` 이 한다.
        let layout = self.layout.0.lock().unwrap_or_else(PoisonError::into_inner);
        StateRevision::of(&layout, tree)
    }

    fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
        let tree = self.tree.attrs();
        // 독 든 모델은 쓰지 않는다 — 패닉한 변경이 반쯤 남았을 수 있고, 쓰면 다음 부팅이 그 모양을 복원한다.
        let layout = self
            .layout
            .0
            .lock()
            .map_err(|_| "레이아웃 락에 독이 들었다".to_string())?;
        Ok(to_persisted(&layout, tree))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::path::PathBuf;

    use super::*;
    use crate::commands::popout::PopupCounter;
    use crate::layout::MAIN_WINDOW_LABEL;
    use crate::state::codec;
    use crate::state::lock::LOCK_FILE;
    use crate::state::saver::SnapshotSource;
    use crate::state::schema::{StateFile, STATE_VERSION};
    use crate::ui_settings::UiTheme;

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

    fn source() -> (LiveSource, LayoutState, Arc<TreeAttrs>) {
        let layout = LayoutState::new();
        let tree = Arc::new(TreeAttrs::default());
        (LiveSource::new(layout.clone(), tree.clone()), layout, tree)
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

    fn boot(layout: &LayoutState, tree: &Arc<TreeAttrs>) -> (Boot, Arc<StateSession>) {
        let session = Arc::new(StateSession::default());
        let boot = Boot {
            layout: layout.clone(),
            tree: tree.clone(),
            labels: Arc::new(PopupCounter::default()),
            session: session.clone(),
        };
        (boot, session)
    }

    // ── 스냅숏 원천 ──

    #[test]
    fn revision_moves_with_layout_attrs_and_tree() {
        let (source, layout, tree) = source();
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

        tree.set_theme(Some(UiTheme::Dark));
        let after_tree = source.revision();
        assert_ne!(after_tree.tree, after_attrs.tree);
        assert_eq!(after_tree.attrs, after_attrs.attrs);
    }

    #[test]
    fn snapshot_carries_the_model_and_the_tree_cell() {
        let (source, layout, tree) = source();
        let view = main_view(&layout);
        layout
            .0
            .lock()
            .unwrap()
            .rename_tab(view, "저장할 탭".into())
            .unwrap();
        tree.set_theme(Some(UiTheme::EInk));

        let windows = source.snapshot().expect("스냅숏");
        let expected = to_persisted(&layout.0.lock().unwrap(), tree.attrs());
        assert_eq!(windows, expected);
    }

    #[test]
    fn a_poisoned_layout_lock_fails_the_snapshot_without_panicking() {
        let (source, layout, _tree) = source();
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
            let tree = WindowAttrs {
                theme: Some(UiTheme::Dark),
                ..WindowAttrs::default()
            };
            let text = codec::encode(&StateFile {
                version: STATE_VERSION,
                saved_at_ms: 1,
                clean_exit: true,
                resolved_crash_copy: None,
                windows: to_persisted(&previous, tree),
            })
            .unwrap();
            std::fs::write(state_dir.join(STATE_FILE), text).unwrap();
            view
        };

        let layout = LayoutState::new();
        let tree = Arc::new(TreeAttrs::default());
        let (boot, session) = boot(&layout, &tree);
        boot.restore(&run_dir, &state_dir);

        {
            let mgr = layout.0.lock().unwrap();
            assert_eq!(mgr.views[&saved_tab].name, "지난 탭");
            assert_eq!(mgr.windows.len(), 2, "main + 복원한 팝아웃");
            assert!(
                mgr.windows.contains_key("slot-popup-1"),
                "팝아웃은 이 실행의 발급기에서 새 label 을 받는다"
            );
        }
        assert_eq!(tree.attrs().theme, Some(UiTheme::Dark));
        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit, "실행 표식");
        assert_eq!(marker.windows.len(), 3, "main · 트리 · 팝아웃");
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
        let tree = Arc::new(TreeAttrs::default());
        let (boot, _session) = boot(&layout, &tree);
        boot.restore(&run_dir, &state_dir);

        let marker = read_state(&state_dir);
        assert!(!marker.clean_exit);
        assert_eq!(
            marker.windows,
            to_persisted(&layout.0.lock().unwrap(), tree.attrs()),
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
        let tree = Arc::new(TreeAttrs::default());
        let (boot, session) = boot(&layout, &tree);
        boot.restore(&run_dir, &state_dir);

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
    fn shutdown_without_a_saver_still_releases_the_lock() {
        let run_dir = temp_dir("run");
        let session = StateSession::default();
        session.hold_lock(lock::acquire(&run_dir));
        assert!(!lock_is_free(&run_dir));

        session.shutdown();
        assert!(lock_is_free(&run_dir));
    }
}
