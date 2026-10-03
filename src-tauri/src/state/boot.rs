//! 부팅 판정 — 부팅 단계(TRD S21-storage §6-5)의 ②–④([`prepare`])와 ⑤ 의 실행 표식 쓰기([`write_run_marker`]).
//! 판정([`decide_boot`])은 순수 함수이고, 디스크는 전부 포트 [`BootFiles`] 뒤에 있다(운영 = [`FsBootFiles`]).
//!
//! - ★어느 단계가 실패해도 계속한다★ — 부팅 단계는 `Err` 를 돌려주지 않는다(N7 · D8). 실패는 여기서 log 한다.
//! - ★예외는 가드 하나([`Guard`] · N3)★ — 그 실행은 상태를 하나도 저장하지 않는다. 덮으면 아직 못 본 화면을
//!   잃는다.
//! - ★한시(P3b2–P3b3): 크래시 사본 열이 없다★ — 비정상 종료 뒤엔 기본 화면으로 시작하고 아무것도 하지 않는다.
//!   사본 읽기 · 뜨기 · 묻기 · 답한 사본 지우기는 P3c1 이 [`BootInputs`] · [`BootAction`] · [`Guard`] ·
//!   [`BootPlan`] 에 칸 · 변형을 더해 넣는다. ④ 의 결과(사본을 못 떠 서는 가드 · 못 지운 답한 사본의 해시)는 ④ 가
//!   계획에 접어 넣고 ⑤([`write_run_marker`])가 그 계획을 읽는다 — 그래서 가드 로그는 ④ 뒤에 낸다.

use std::io;
use std::path::{Path, PathBuf};

use super::codec::{self, DecodeWarning, Unusable, STATE_READ_CAP};
use super::schema::{StateFile, WindowEntry, STATE_VERSION};

pub const STATE_FILE: &str = "state.json";
pub const CRASH_COPY_FILE: &str = "state.crash.json";

/// 부팅 단계가 만지는 파일 — 운영은 [`FsBootFiles`].
pub trait BootFiles {
    /// `state.json` 원문 — `crate::fsutil::read_file_capped` 와 같은 계약(없으면 `NotFound` · 상한 초과 · UTF-8
    /// 아님은 `InvalidData` · 잠김은 재시도 뒤의 그 오류).
    fn read_state(&self) -> io::Result<String>;

    /// `state.json` 을 `state.json.corrupt` 로 떠 둔다 — 돌려주는 값 = 사본 자리(`crate::fsutil::copy_aside`).
    fn copy_aside_state(&self) -> io::Result<PathBuf>;

    /// `state.json` 을 원자적으로 갈아끼운다 — 폴더가 없으면 만든다.
    fn write_state(&self, text: &str) -> io::Result<()>;

    /// 상태 파일들의 남은 임시 파일을 치운다 — `crate::fsutil::sweep_temps` 와 같은 계약.
    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>>;
}

/// 운영 파일 — `<dir>\state.json` 과 그 옆. `dir` = 셸 state 폴더(배치는 P3b3 · `DataLayout`).
pub struct FsBootFiles {
    dir: PathBuf,
}

impl FsBootFiles {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn state(&self) -> PathBuf {
        self.dir.join(STATE_FILE)
    }
}

impl BootFiles for FsBootFiles {
    fn read_state(&self) -> io::Result<String> {
        crate::fsutil::read_file_capped(&self.state(), STATE_READ_CAP)
    }

    fn copy_aside_state(&self) -> io::Result<PathBuf> {
        crate::fsutil::copy_aside(&self.state())
    }

    fn write_state(&self, text: &str) -> io::Result<()> {
        // 셸 state 폴더는 아무도 미리 만들지 않는다 — 첫 부팅의 첫 쓰기가 만든다. 기록기는 만들지 않는다.
        std::fs::create_dir_all(&self.dir)?;
        crate::fsutil::write_atomic(&self.state(), text)
    }

    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
        crate::fsutil::sweep_temps(
            &self.dir,
            &[STATE_FILE, CRASH_COPY_FILE],
            engram_dashboard_base::platform::pid_alive,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StateRead {
    Missing,
    /// 내용이 아니라 읽기 자체가 실패했다(잠김 재시도 뒤 · 권한) — ★못 쓸 파일과 섞지 않는다★: 잠깐 잠긴 멀쩡한
    /// 파일을 떠 두거나 덮지 않는다(I3).
    IoFailed(String),
    Unusable(Unusable),
    /// `text` = 읽은 원문 그대로 — 크래시 사본은 디스크를 다시 읽지 않고 이 바이트로 뜬다(P3c1 · §6-5 ④ · L2).
    Usable {
        file: StateFile,
        text: String,
        warnings: Vec<DecodeWarning>,
    },
}

impl StateRead {
    pub fn from_read(read: io::Result<String>) -> Self {
        match read {
            Ok(text) => match codec::decode(&text) {
                Ok((file, warnings)) => StateRead::Usable {
                    file,
                    text,
                    warnings,
                },
                Err(unusable) => StateRead::Unusable(unusable),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => StateRead::Missing,
            Err(e) => match codec::unusable_read(&e) {
                Some(unusable) => StateRead::Unusable(unusable),
                None => StateRead::IoFailed(e.to_string()),
            },
        }
    }
}

/// [`decide_boot`] 의 입력 — 부팅 단계 ②가 본 것 전부.
#[derive(Debug, Clone, PartialEq)]
pub struct BootInputs {
    pub state: StateRead,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BootModel {
    /// 기본 레이아웃(ADR-0222) — 복원할 상태가 없다.
    Default,
    /// 조용히 복원한다(「항상 복원」).
    Restore(StateFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootAction {
    /// 못 쓸 `state.json` 을 `state.json.corrupt` 로 떠 둔다(ADR-0274) — 원본은 실행 표식 쓰기(⑤)가 갈아끼운다.
    /// 실패해도 진행한다(D8).
    CopyAsideState,
}

/// 이번 실행은 상태를 하나도 저장하지 않는다(N3) — 실행 표식 쓰기(⑤)를 건너뛰고 기록기를 띄우지 않으며, 종료는
/// `Final` 없이 잠금만 놓는다(N7). 다음 부팅이 다시 판정한다(§12 R16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// `state.json` 을 못 읽었다(IO — 잠김 재시도 뒤). 사유 한 줄.
    StateUnreadable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BootPlan {
    pub model: BootModel,
    /// 부팅 단계 ④가 이 순서대로 한다.
    pub actions: Vec<BootAction>,
    /// 판정(③)이나 동작(④)이 세운다 — ⑤ 는 ④ 뒤의 값을 본다.
    pub guard: Option<Guard>,
}

/// §6-5 표 — 지금은 「사본 없음」 열만 있다(모듈 머리 「한시」).
pub fn decide_boot(inputs: BootInputs) -> BootPlan {
    let mut plan = BootPlan {
        model: BootModel::Default,
        actions: Vec::new(),
        guard: None,
    };
    match inputs.state {
        StateRead::Missing => {}
        // 「없음」으로 치되 그 부팅의 동작은 하나도 하지 않는다(I3).
        StateRead::IoFailed(reason) => plan.guard = Some(Guard::StateUnreadable(reason)),
        // 버전 초과(새 판이 쓴 파일 — 하향)도 떠 둔다(§12 R3).
        StateRead::Unusable(_) => plan.actions.push(BootAction::CopyAsideState),
        StateRead::Usable { file, .. } if file.clean_exit => plan.model = BootModel::Restore(file),
        // TODO(P3c1): 원문(`text`)으로 크래시 사본을 뜨고 묻는다(§6-5 표 「없음 | clean_exit:false」 · D5) — 그때
        //   사본 열이 이 match 를 다시 짠다.
        StateRead::Usable { .. } => {}
    }
    plan
}

/// 부팅 단계 ②–④ — 남은 임시 파일을 쓸고, `state.json` 을 읽어 판정하고, 판정한 동작을 한다. 실패는 log 하고
/// 계속한다.
///
/// ★셸 실행 잠금([`super::lock::acquire`]) 뒤 · 기록기 앞에 부른다★ — 잠금 뒤라야 앞 인스턴스의 마지막 쓰기를
/// 읽고, 기록기 앞이라야 자기 pid 의 임시 파일을 지워도 된다.
pub fn prepare(files: &impl BootFiles) -> BootPlan {
    sweep(files);
    let state = StateRead::from_read(files.read_state());
    report_read(&state);
    let mut plan = decide_boot(BootInputs { state });
    tracing::info!(
        module = "state",
        restore = matches!(plan.model, BootModel::Restore(_)),
        actions = ?plan.actions,
        "부팅 판정"
    );
    run_actions(files, &mut plan);
    if let Some(guard) = &plan.guard {
        tracing::warn!(
            module = "state",
            guard = ?guard,
            "이번 실행은 화면 상태를 저장하지 않는다 — 다음 부팅이 다시 판정한다"
        );
    }
    plan
}

fn sweep(files: &impl BootFiles) {
    match files.sweep_temps() {
        Ok(swept) => {
            for (path, removed) in swept {
                match removed {
                    Ok(()) => tracing::debug!(
                        module = "state",
                        path = %path.display(),
                        "남은 임시 파일을 지웠다"
                    ),
                    Err(e) => tracing::warn!(
                        module = "state",
                        path = %path.display(),
                        error = %e,
                        "남은 임시 파일을 못 지웠다"
                    ),
                }
            }
        }
        Err(e) => tracing::warn!(
            module = "state",
            error = %e,
            "남은 임시 파일을 찾지 못했다 — 치우지 않고 진행한다"
        ),
    }
}

fn report_read(state: &StateRead) {
    match state {
        StateRead::Missing => {}
        StateRead::IoFailed(reason) => tracing::warn!(
            module = "state",
            reason = %reason,
            "state.json 을 읽지 못했다 — 없는 것으로 친다"
        ),
        StateRead::Unusable(unusable) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.json 을 못 쓴다 — 기본 화면으로 시작한다(떠 둔 뒤 실행 표식 쓰기가 갈아끼운다)"
        ),
        StateRead::Usable { file, warnings, .. } => {
            warnings.iter().for_each(report_decode_warning);
            if !file.clean_exit {
                tracing::warn!(
                    module = "state",
                    "앞 실행이 정상 종료를 적지 못했다 — 기본 화면으로 시작한다"
                );
            }
        }
    }
}

fn report_decode_warning(warning: &DecodeWarning) {
    match warning {
        DecodeWarning::WindowSkipped { index, id, reason } => tracing::warn!(
            module = "state",
            window_index = index,
            window_id = ?id,
            reason = %reason,
            "state.json 의 창 하나를 건너뛴다"
        ),
        DecodeWarning::TabSkipped {
            window_id,
            index,
            reason,
        } => tracing::warn!(
            module = "state",
            window_id = %window_id,
            tab_index = index,
            reason = %reason,
            "state.json 의 탭 하나를 건너뛴다"
        ),
        DecodeWarning::ThemeDropped { window_id, raw } => tracing::warn!(
            module = "state",
            window_id = %window_id,
            raw = %raw,
            "state.json 의 창 테마를 모른다 — 전역 테마를 따른다"
        ),
    }
}

/// 부팅 단계 ④ — 동작의 결과 중 ⑤ 가 알아야 할 것은 `plan` 에 접어 넣는다(모듈 머리 「한시」).
fn run_actions(files: &impl BootFiles, plan: &mut BootPlan) {
    // 복사본을 돈다 — 동작마다 `plan` 의 다른 칸을 고칠 수 있어야 한다.
    for action in plan.actions.clone() {
        match action {
            BootAction::CopyAsideState => match files.copy_aside_state() {
                Ok(to) => tracing::info!(
                    module = "state",
                    to = %to.display(),
                    "못 쓰는 state.json 을 떠 뒀다"
                ),
                Err(e) => tracing::error!(
                    module = "state",
                    error = %e,
                    "못 쓰는 state.json 을 떠 두지 못했다 — 진행한다: 실행 표식 쓰기가 하나뿐인 원본을 덮는다(D8)"
                ),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerOutcome {
    Written,
    /// 가드라 쓰지 않았다.
    Guarded,
    /// 코덱 거절 · 디스크 오류 — log 했다.
    Failed,
}

/// 부팅 단계 ⑤ — 실행 표식(`clean_exit:false`)과 판정한 모델의 창들을 `state.json` 에 직접 쓴다. 기록기는 아직 없다.
///
/// ★어느 창보다 먼저 부른다★ — 조용히 복원한 화면이 창을 만들다 앱을 죽여도 다음 부팅이 비정상 종료로 읽는다
/// (§6-5 ⑤). 실패는 log 만 하고 진행한다 — 그 실행의 크래시는 정상 종료로 읽힐 수 있다(D8 · §12 R14).
pub fn write_run_marker(
    files: &impl BootFiles,
    plan: &BootPlan,
    windows: Vec<WindowEntry>,
    saved_at_ms: u64,
) -> MarkerOutcome {
    if plan.guard.is_some() {
        return MarkerOutcome::Guarded;
    }
    let marker = StateFile {
        version: STATE_VERSION,
        saved_at_ms,
        clean_exit: false,
        // TODO(P3c1): 못 지운 답한 사본의 해시(`carry_resolved`)를 싣는다.
        resolved_crash_copy: None,
        windows,
    };
    let text = match codec::encode(&marker) {
        Ok(text) => text,
        Err(e) => {
            tracing::error!(module = "state", error = %e, "실행 표식을 쓰지 못했다");
            return MarkerOutcome::Failed;
        }
    };
    match files.write_state(&text) {
        Ok(()) => MarkerOutcome::Written,
        Err(e) => {
            tracing::warn!(module = "state", error = %e, "실행 표식을 쓰지 못했다");
            MarkerOutcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::state::schema::{Bounds, WindowKind};

    fn state_file(clean_exit: bool) -> StateFile {
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 42,
            clean_exit,
            resolved_crash_copy: None,
            windows: vec![tree_window()],
        }
    }

    fn tree_window() -> WindowEntry {
        WindowEntry {
            id: "agent-tree".into(),
            kind: WindowKind::Tree,
            theme: None,
            bounds: Some(Bounds {
                x: 10.0,
                y: 20.0,
                w: 300.0,
                h: 400.0,
            }),
            maximized: false,
        }
    }

    fn usable(clean_exit: bool) -> StateRead {
        let file = state_file(clean_exit);
        StateRead::Usable {
            text: codec::encode(&file).unwrap(),
            file,
            warnings: Vec::new(),
        }
    }

    fn decide(state: StateRead) -> BootPlan {
        decide_boot(BootInputs { state })
    }

    fn quiet(model: BootModel) -> BootPlan {
        BootPlan {
            model,
            actions: Vec::new(),
            guard: None,
        }
    }

    // ── 판정표(사본 없음 열) ──

    #[test]
    fn no_state_file_starts_the_default_screen_quietly() {
        assert_eq!(decide(StateRead::Missing), quiet(BootModel::Default));
    }

    #[test]
    fn a_clean_exit_restores_quietly() {
        assert_eq!(
            decide(usable(true)),
            quiet(BootModel::Restore(state_file(true)))
        );
    }

    #[test]
    fn an_unclean_exit_starts_the_default_screen_and_does_nothing_else_for_now() {
        assert_eq!(decide(usable(false)), quiet(BootModel::Default));
    }

    #[test]
    fn an_unusable_state_file_is_copied_aside_and_starts_the_default_screen() {
        for unusable in [
            Unusable::Unreadable("상한 초과".into()),
            Unusable::NotJson("{".into()),
            Unusable::NotStateFile("머리 없음".into()),
            Unusable::NewerVersion { found: 2 },
        ] {
            assert_eq!(
                decide(StateRead::Unusable(unusable.clone())),
                BootPlan {
                    model: BootModel::Default,
                    actions: vec![BootAction::CopyAsideState],
                    guard: None,
                },
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn a_read_io_failure_guards_the_run_and_does_nothing() {
        assert_eq!(
            decide(StateRead::IoFailed("잠김".into())),
            BootPlan {
                model: BootModel::Default,
                actions: Vec::new(),
                guard: Some(Guard::StateUnreadable("잠김".into())),
            }
        );
    }

    // ── 읽기 결과 가르기 ──

    #[test]
    fn read_errors_split_into_missing_unusable_and_io_failure() {
        let from_err = |e: io::Error| StateRead::from_read(Err(e));
        assert_eq!(from_err(io::ErrorKind::NotFound.into()), StateRead::Missing);
        assert!(matches!(
            from_err(io::Error::new(io::ErrorKind::InvalidData, "상한 초과")),
            StateRead::Unusable(Unusable::Unreadable(_))
        ));
        assert!(matches!(
            from_err(io::ErrorKind::PermissionDenied.into()),
            StateRead::IoFailed(_)
        ));
        assert!(matches!(
            from_err(io::Error::from_raw_os_error(32)),
            StateRead::IoFailed(_)
        ));
    }

    #[test]
    fn read_text_splits_into_unusable_and_usable() {
        assert!(matches!(
            StateRead::from_read(Ok("{".into())),
            StateRead::Unusable(Unusable::NotJson(_))
        ));
        assert_eq!(
            StateRead::from_read(Ok(
                r#"{"version":2,"saved_at_ms":1,"clean_exit":true,"windows":[]}"#.into()
            )),
            StateRead::Unusable(Unusable::NewerVersion { found: 2 })
        );
        let text = codec::encode(&state_file(true)).unwrap();
        assert_eq!(
            StateRead::from_read(Ok(text.clone())),
            StateRead::Usable {
                file: state_file(true),
                text,
                warnings: Vec::new(),
            }
        );
    }

    // ── 부팅 단계 ②–④ · ⑤(가짜 파일) ──

    #[derive(Default)]
    struct FakeFiles {
        state: RefCell<Option<io::Result<String>>>,
        copy_aside_fails: bool,
        write_fails: bool,
        sweep_fails: bool,
        calls: RefCell<Vec<String>>,
    }

    impl FakeFiles {
        fn reading(read: io::Result<String>) -> Self {
            Self {
                state: RefCell::new(Some(read)),
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn written(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter_map(|call| call.strip_prefix("write:").map(str::to_string))
                .collect()
        }
    }

    impl BootFiles for FakeFiles {
        fn read_state(&self) -> io::Result<String> {
            self.calls.borrow_mut().push("read".into());
            self.state
                .borrow_mut()
                .take()
                .expect("부팅은 state.json 을 한 번 읽는다")
        }

        fn copy_aside_state(&self) -> io::Result<PathBuf> {
            self.calls.borrow_mut().push("copy_aside".into());
            if self.copy_aside_fails {
                Err(io::Error::other("가짜 떠 두기 실패"))
            } else {
                Ok(PathBuf::from("state.json.corrupt"))
            }
        }

        fn write_state(&self, text: &str) -> io::Result<()> {
            self.calls.borrow_mut().push(format!("write:{text}"));
            if self.write_fails {
                Err(io::Error::other("가짜 쓰기 실패"))
            } else {
                Ok(())
            }
        }

        fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
            self.calls.borrow_mut().push("sweep".into());
            if self.sweep_fails {
                Err(io::Error::other("가짜 폴더 읽기 실패"))
            } else {
                Ok(vec![
                    (PathBuf::from("state.json.tmp1.0"), Ok(())),
                    (
                        PathBuf::from("state.json.tmp2.0"),
                        Err(io::Error::other("가짜 지우기 실패")),
                    ),
                ])
            }
        }
    }

    #[test]
    fn an_unusable_state_file_is_copied_aside_once_after_the_sweep_and_the_read() {
        let files = FakeFiles::reading(Ok("{".into()));

        let plan = prepare(&files);

        assert_eq!(plan.model, BootModel::Default);
        assert_eq!(plan.guard, None);
        assert_eq!(files.calls(), ["sweep", "read", "copy_aside"]);
    }

    #[test]
    fn a_failed_copy_aside_is_logged_and_the_boot_goes_on_to_write_the_marker() {
        let files = FakeFiles {
            copy_aside_fails: true,
            ..FakeFiles::reading(Ok("{".into()))
        };

        let plan = prepare(&files);

        assert_eq!(plan.guard, None, "떠 두기 실패는 가드가 아니다(D8)");
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 7),
            MarkerOutcome::Written
        );
        assert_eq!(files.written().len(), 1);
    }

    #[test]
    fn a_read_io_failure_copies_nothing_and_writes_nothing_this_run() {
        let files = FakeFiles::reading(Err(io::ErrorKind::PermissionDenied.into()));

        let plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::StateUnreadable(_))));
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 7),
            MarkerOutcome::Guarded
        );
        assert_eq!(files.calls(), ["sweep", "read"], "떠 두기 · 쓰기 0회");
    }

    #[test]
    fn a_clean_exit_restores_without_touching_the_disk_beyond_the_sweep_and_the_read() {
        let files = FakeFiles::reading(Ok(codec::encode(&state_file(true)).unwrap()));

        let plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Restore(state_file(true))));
        assert_eq!(files.calls(), ["sweep", "read"]);
    }

    #[test]
    fn a_failed_sweep_does_not_stop_the_read() {
        let files = FakeFiles {
            sweep_fails: true,
            ..FakeFiles::reading(Err(io::ErrorKind::NotFound.into()))
        };

        let plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Default));
        assert_eq!(files.calls(), ["sweep", "read"]);
    }

    #[test]
    fn the_run_marker_is_an_unclean_exit_with_the_given_windows() {
        let files = FakeFiles::default();

        let outcome = write_run_marker(
            &files,
            &quiet(BootModel::Restore(state_file(true))),
            vec![tree_window()],
            7,
        );

        assert_eq!(outcome, MarkerOutcome::Written);
        let written = files.written();
        let (file, warnings) = codec::decode(&written[0]).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            file,
            StateFile {
                version: STATE_VERSION,
                saved_at_ms: 7,
                clean_exit: false,
                resolved_crash_copy: None,
                windows: vec![tree_window()],
            }
        );
    }

    #[test]
    fn a_failed_marker_write_is_reported_not_raised() {
        let files = FakeFiles {
            write_fails: true,
            ..FakeFiles::default()
        };

        let outcome = write_run_marker(&files, &quiet(BootModel::Default), Vec::new(), 7);

        assert_eq!(outcome, MarkerOutcome::Failed);
    }

    // ── 운영 파일(임시 폴더) ──

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "engram-state-boot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ))
    }

    #[test]
    fn a_first_boot_without_a_state_folder_writes_the_marker_into_a_new_folder() {
        let root = temp_dir("first");
        let dir = root.join("shell").join("state");
        let files = FsBootFiles::in_dir(&dir);

        let plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Default));
        assert_eq!(
            write_run_marker(&files, &plan, Vec::new(), 7),
            MarkerOutcome::Written
        );
        let text = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(!codec::decode(&text).unwrap().0.clean_exit);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unusable_state_file_on_disk_is_kept_aside_before_the_marker_replaces_it() {
        let dir = temp_dir("unusable");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STATE_FILE), "{broken").unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let plan = prepare(&files);
        write_run_marker(&files, &plan, Vec::new(), 7);

        assert_eq!(
            std::fs::read_to_string(dir.join("state.json.corrupt")).unwrap(),
            "{broken"
        );
        let text = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(codec::decode(&text).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_boot_sweeps_its_own_leftover_temps_and_leaves_other_files() {
        let dir = temp_dir("sweep");
        std::fs::create_dir_all(&dir).unwrap();
        let me = std::process::id();
        let own = [
            format!("state.json.tmp{me}.0"),
            format!("state.crash.json.corrupt.tmp{me}.1"),
        ];
        let others = ["settings.json.tmp1.0", "state.json.tmpX"];
        for name in own.iter().map(String::as_str).chain(others) {
            std::fs::write(dir.join(name), "x").unwrap();
        }

        prepare(&FsBootFiles::in_dir(&dir));

        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, others);
        std::fs::remove_dir_all(&dir).ok();
    }
}
