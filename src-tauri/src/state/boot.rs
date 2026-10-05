//! 부팅 판정 — 부팅 단계(TRD S21-storage §6-5)의 ②–④([`prepare`])와 ⑤ 의 실행 표식 쓰기([`write_run_marker`]).
//! 판정([`decide_boot`])은 순수 함수이고, 디스크는 전부 포트 [`BootFiles`] 뒤에 있다(운영 = [`FsBootFiles`]).
//!
//! - ★어느 단계가 실패해도 계속한다★ — 부팅 단계는 `Err` 를 돌려주지 않는다(N7 · D8). 실패는 여기서 log 한다.
//! - ★예외는 가드 하나([`Guard`] · N3)★ — 그 실행은 상태를 하나도 저장하지 않는다. 덮으면 아직 사본으로 뜨지
//!   못했을지 모르는 크래시 화면을 잃는다.
//! - ★답하지 않은 크래시 사본은 덮지도 지우지도 않는다(D2-6)★ — 이 판이 못 쓰는 새 판의 사본도(N6). 부팅이 사본을
//!   지우는 것은 못 쓸 사본(떠 둔 뒤)과 답한 사본(다시 읽어 해시가 같을 때만 — N1)뿐이다.
//! - ④ 의 결과(사본을 못 떠 서는 가드 · 못 지운 답한 사본의 해시 · 못 쓸 `state.json` 을 떠 두었나)는 ④ 가 계획에
//!   접어 넣고, ⑤([`write_run_marker`]) · ⑥ · 기록기가 그 계획을 읽는다 — 그래서 가드 로그는 ④ 뒤에 낸다.

use std::fmt;
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

    /// `state.crash.json` 원문 — [`Self::read_state`] 와 같은 계약.
    fn read_crash_copy(&self) -> io::Result<String>;

    /// `state.crash.json` 을 `text` 로 원자적으로 갈아끼운다 — 폴더가 없으면 만든다.
    fn write_crash_copy(&self, text: &str) -> io::Result<()>;

    /// 없으면 `NotFound`.
    fn remove_crash_copy(&self) -> io::Result<()>;

    /// `state.crash.json` 을 `state.crash.json.corrupt` 로 떠 둔다 — [`Self::copy_aside_state`] 와 같은 계약.
    fn copy_aside_crash_copy(&self) -> io::Result<PathBuf>;

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

    fn crash_copy(&self) -> PathBuf {
        self.dir.join(CRASH_COPY_FILE)
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

    fn read_crash_copy(&self) -> io::Result<String> {
        crate::fsutil::read_file_capped(&self.crash_copy(), STATE_READ_CAP)
    }

    fn write_crash_copy(&self, text: &str) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        crate::fsutil::write_atomic(&self.crash_copy(), text)
    }

    // 잠김 재시도를 하지 않는다 — 못 지운 답한 사본은 기록기가 해시를 이어 받아 다시 지우고, 못 쓸 사본은 다음
    //   부팅이 다시 지운다.
    fn remove_crash_copy(&self) -> io::Result<()> {
        std::fs::remove_file(self.crash_copy())
    }

    fn copy_aside_crash_copy(&self) -> io::Result<PathBuf> {
        crate::fsutil::copy_aside(&self.crash_copy())
    }

    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
        crate::fsutil::sweep_temps(
            &self.dir,
            &[STATE_FILE, CRASH_COPY_FILE],
            engram_dashboard_base::platform::pid_alive,
        )
    }
}

/// 상태 파일 하나(`state.json` 또는 같은 모양의 크래시 사본)를 읽은 결과.
#[derive(Clone, PartialEq)]
pub enum StateRead {
    Missing,
    /// 내용이 아니라 읽기 자체가 실패했다(잠김 재시도 뒤 · 권한) — ★못 쓸 파일과 섞지 않는다★: 잠깐 잠긴 멀쩡한
    /// 파일을 떠 두거나 덮지 않는다(I3).
    IoFailed(String),
    Unusable(Unusable),
    /// `text` = 읽은 원문 그대로 — 사본 뜨기 · 사본 해시 · 수락의 원천이 이 바이트다(디스크를 다시 읽지 않는다 —
    /// §6-5 ④ · L2).
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

// 원문(≤4 MiB)을 찍지 않는다 — 길이만.
impl fmt::Debug for StateRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateRead::Missing => f.write_str("Missing"),
            StateRead::IoFailed(reason) => f.debug_tuple("IoFailed").field(reason).finish(),
            StateRead::Unusable(unusable) => f.debug_tuple("Unusable").field(unusable).finish(),
            StateRead::Usable {
                file,
                text,
                warnings,
            } => f
                .debug_struct("Usable")
                .field("text_len", &text.len())
                .field("clean_exit", &file.clean_exit)
                .field("windows", &file.windows.len())
                .field("warnings", &warnings.len())
                .finish(),
        }
    }
}

/// [`decide_boot`] 의 입력 — 부팅 단계 ②가 본 것 전부.
#[derive(Debug, Clone, PartialEq)]
pub struct BootInputs {
    pub state: StateRead,
    pub crash_copy: StateRead,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BootModel {
    /// 기본 레이아웃(ADR-0222) — 복원할 상태가 없거나 묻는다.
    Default,
    /// 조용히 복원한다(「항상 복원」).
    Restore(StateFile),
}

/// 복원할지 묻는 원문 — ⑥ 이 복원 서비스에 넘긴다. 답 없는 사본이 있으면 그 사본이고, 이 부팅이 뜨는 사본이면
/// `state.json` 원문이다 — 뜨기에 실패해도(가드 ⅱ) 같은 원문을 메모리에서 쓴다(L2).
#[derive(Clone, PartialEq)]
pub struct AwaitingCopy {
    pub text: String,
    /// `codec::crash_copy_hash(&text)`.
    pub hash: String,
    pub file: StateFile,
}

// 원문(≤4 MiB)을 찍지 않는다 — 길이와 해시만.
impl fmt::Debug for AwaitingCopy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwaitingCopy")
            .field("text_len", &self.text.len())
            .field("hash", &self.hash)
            .field("windows", &self.file.windows.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootAction {
    /// 못 쓸 `state.json` 을 `state.json.corrupt` 로 떠 둔다(ADR-0274) — 원본은 실행 표식 쓰기(⑤)가 갈아끼운다.
    /// 실패해도 진행한다(D8).
    CopyAsideState,
    /// 못 쓸 사본(버전 초과 말고)을 `state.crash.json.corrupt` 로 떠 둔다(ADR-0274). 실패해도 진행한다(D8).
    CopyAsideCrashCopy,
    /// 떠 둔 못 쓸 사본을 지운다 — ★떠 두기가 실패했으면 지우지 않는다★: 둘 곳이 없는 원본을 남겨 다음 부팅이
    /// 다시 떠 둔다.
    RemoveUnusableCrashCopy,
    /// `state.json` 원문([`BootPlan::crash_copy`])을 사본으로 뜬다 — 있던 사본(답한 · 못 쓸)은 이 쓰기가 갈아끼운다.
    /// 실패 = 가드 ⅱ.
    WriteCrashCopy,
    /// 답한 사본을 다시 읽어(≤4 MiB) 해시가 같을 때만 지운다(N1). 못 지웠거나 답한 사본인지 모르면 해시를 넘긴다
    /// ([`BootPlan::carry_resolved`]).
    RemoveAnsweredCrashCopy { hash: String },
}

/// 이번 실행은 상태를 하나도 저장하지 않는다(N3) — 실행 표식 쓰기(⑤)를 건너뛰고 기록기를 띄우지 않으며, 종료는
/// `Final` 없이 잠금만 놓는다(N7). 다음 부팅이 다시 판정한다(§12 R16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// ⅰ `state.json` 을 못 읽었다(IO — 잠김 재시도 뒤). 사유 한 줄.
    StateUnreadable(String),
    /// ⅱ 떠야 할 사본을 못 떴다 — 쓰기 실패 · 사본 읽기 IO 실패 · 덮으면 안 되는 새 판의 사본(N6). 사유 한 줄.
    CrashCopyNotWritten(String),
}

/// [`BootAction::CopyAsideState`] 의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateAside {
    CopiedAside,
    /// 원본은 실행 표식 쓰기(⑤)가 백업 없이 갈아끼운다(D8).
    NotCopied,
}

#[derive(Clone, PartialEq)]
pub struct BootPlan {
    pub model: BootModel,
    /// 부팅 단계 ④가 이 순서대로 한다.
    pub actions: Vec<BootAction>,
    /// `Some` = 묻는다(⑥ 의 `awaiting`). 가드 ⅱ 여도 선다.
    pub crash_copy: Option<AwaitingCopy>,
    /// ⑤ 와 기록기가 이어 실을 답한 사본의 해시 — 그 사본이 디스크에 남아 있을 수 있을 때만 선다(판정 · ④).
    pub carry_resolved: Option<String>,
    /// 판정(③)이나 동작(④)이 세운다 — ⑤ 는 ④ 뒤의 값을 본다.
    pub guard: Option<Guard>,
    /// `Some` = `state.json` 이 못 쓸 파일이었다 — 떠 두기의 결과다. 동작(④)이 세운다(판정 뒤엔 아직 `None`).
    pub state_aside: Option<StateAside>,
}

// 복원할 모델(원문 ≤4 MiB 만큼의 탭 트리)을 통째로 찍지 않는다 — 창 수만.
impl fmt::Debug for BootPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Model<'a>(&'a BootModel);
        impl fmt::Debug for Model<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.0 {
                    BootModel::Default => f.write_str("Default"),
                    BootModel::Restore(file) => f
                        .debug_struct("Restore")
                        .field("windows", &file.windows.len())
                        .finish(),
                }
            }
        }
        f.debug_struct("BootPlan")
            .field("model", &Model(&self.model))
            .field("actions", &self.actions)
            .field("crash_copy", &self.crash_copy)
            .field("carry_resolved", &self.carry_resolved)
            .field("guard", &self.guard)
            .field("state_aside", &self.state_aside)
            .finish()
    }
}

/// 사본 열 — 답 없는 사본 말고는 「없음」으로 치고 `state.json` 행대로 간다(§6-5 표).
enum CopyColumn {
    Unanswered(AwaitingCopy),
    AsAbsent(AbsentCopy),
}

/// 「없음」으로 치는 사본이 디스크에 무엇으로 있나 — 할 정리가 갈린다.
enum AbsentCopy {
    Missing,
    /// 읽기 IO 실패(I3).
    Unread,
    /// 새 판이 쓴 사본 — 지우지도 떠 두지도 덮지도 않는다(N6).
    Newer,
    /// 버전 초과 말고 못 쓸 파일.
    Unusable,
    /// 해시 = `state.json` 의 `resolved_crash_copy`(I2).
    Answered(String),
}

impl CopyColumn {
    fn of(read: StateRead, resolved: Option<&str>) -> Self {
        let absent = match read {
            StateRead::Missing => AbsentCopy::Missing,
            StateRead::IoFailed(_) => AbsentCopy::Unread,
            StateRead::Unusable(Unusable::NewerVersion { .. }) => AbsentCopy::Newer,
            StateRead::Unusable(_) => AbsentCopy::Unusable,
            StateRead::Usable { file, text, .. } => {
                let hash = codec::crash_copy_hash(&text);
                if resolved == Some(hash.as_str()) {
                    AbsentCopy::Answered(hash)
                } else {
                    return CopyColumn::Unanswered(AwaitingCopy { text, hash, file });
                }
            }
        };
        CopyColumn::AsAbsent(absent)
    }
}

/// §6-5 표 — 사본 열 × `state.json` 열.
pub fn decide_boot(inputs: BootInputs) -> BootPlan {
    let BootInputs { state, crash_copy } = inputs;
    let mut plan = BootPlan {
        model: BootModel::Default,
        actions: Vec::new(),
        crash_copy: None,
        carry_resolved: None,
        guard: None,
        state_aside: None,
    };
    // I3 는 파일마다 가른다 — 읽기 IO 실패한 파일만 떠 두지도 덮지도 지우지도 않고, 다른 파일의 동작은 그 파일의
    //   읽기만 본다. 둘을 묶으면 사본을 못 읽은 부팅이 못 쓸 `state.json` 을 떠 두지 않은 채 ⑤ 로 덮는다.

    // 답했다는 사실은 읽을 수 있는 `state.json` 에만 있다 — 못 읽으면 남은 사본은 답 없는 사본이다.
    let resolved = match &state {
        StateRead::Usable { file, .. } => file.resolved_crash_copy.clone(),
        _ => None,
    };

    match &state {
        StateRead::IoFailed(reason) => plan.guard = Some(Guard::StateUnreadable(reason.clone())),
        // 사본 열과 무관하게 떠 둔다(N6 · 사본을 못 읽었어도 — ⑤ 가 이 원본을 갈아끼운다). 버전 초과(새 판이 쓴
        //   파일 — 하향)도 떠 둔다(§12 R3).
        StateRead::Unusable(_) => plan.actions.push(BootAction::CopyAsideState),
        _ => {}
    }

    let copy = match CopyColumn::of(crash_copy, resolved.as_deref()) {
        // D2-6 · D4 — `state.json` 이 무엇이든 기본 화면으로 묻고 사본은 덮지 않는다.
        CopyColumn::Unanswered(awaiting) => {
            plan.crash_copy = Some(awaiting);
            return plan;
        }
        CopyColumn::AsAbsent(copy) => copy,
    };

    match state {
        StateRead::Usable { file, .. } if file.clean_exit => plan.model = BootModel::Restore(file),
        // D5 — 앞 실행의 화면을 사본으로 뜨고 묻는다(모양 무관).
        StateRead::Usable { file, text, .. } => {
            plan.crash_copy = Some(AwaitingCopy {
                hash: codec::crash_copy_hash(&text),
                text,
                file,
            });
            match copy {
                AbsentCopy::Newer => {
                    plan.guard = Some(Guard::CrashCopyNotWritten(
                        "새 판이 쓴 사본이 그 자리에 있어 덮지 않는다".to_string(),
                    ))
                }
                AbsentCopy::Unread => {
                    plan.guard = Some(Guard::CrashCopyNotWritten(
                        "사본을 못 읽어 덮어도 되는지 모른다".to_string(),
                    ))
                }
                AbsentCopy::Unusable => plan
                    .actions
                    .extend([BootAction::CopyAsideCrashCopy, BootAction::WriteCrashCopy]),
                // 답한 사본은 이 쓰기가 갈아끼운다 — 이어 실을 해시가 없다.
                AbsentCopy::Missing | AbsentCopy::Answered(_) => {
                    plan.actions.push(BootAction::WriteCrashCopy)
                }
            }
            return plan;
        }
        StateRead::Missing | StateRead::IoFailed(_) | StateRead::Unusable(_) => {}
    }

    match copy {
        AbsentCopy::Unusable => plan.actions.extend([
            BootAction::CopyAsideCrashCopy,
            BootAction::RemoveUnusableCrashCopy,
        ]),
        AbsentCopy::Answered(hash) => plan
            .actions
            .push(BootAction::RemoveAnsweredCrashCopy { hash }),
        // 답한 사본이 남았는지 볼 수 없다 — 해시를 이어 싣는다. 버리면 아직 남았을지 모르는 그 사본을 다음 부팅이
        //   답 없는 사본으로 읽고 다시 묻는다.
        AbsentCopy::Unread => plan.carry_resolved = resolved,
        AbsentCopy::Missing | AbsentCopy::Newer => {}
    }
    plan
}

/// 부팅 단계 ②–④ — 남은 임시 파일을 쓸고, `state.json` 과 크래시 사본을 읽어 판정하고, 판정한 동작을 한다.
/// 실패는 log 하고 계속한다.
///
/// ★셸 실행 잠금([`super::lock::acquire`]) 뒤 · 기록기 앞에 부른다★ — 잠금 뒤라야 앞 인스턴스의 마지막 쓰기를
/// 읽고, 기록기 앞이라야 자기 pid 의 임시 파일을 지워도 된다.
pub fn prepare(files: &impl BootFiles) -> BootPlan {
    sweep(files);
    let state = StateRead::from_read(files.read_state());
    report_state_read(&state);
    let crash_copy = StateRead::from_read(files.read_crash_copy());
    report_crash_copy_read(&crash_copy);
    let mut plan = decide_boot(BootInputs { state, crash_copy });
    tracing::info!(
        module = "state",
        restore = matches!(plan.model, BootModel::Restore(_)),
        ask = plan.crash_copy.is_some(),
        actions = ?plan.actions,
        carry_resolved = ?plan.carry_resolved,
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

fn report_state_read(state: &StateRead) {
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
            "state.json 을 못 쓴다 — 기본 화면으로 시작한다(실행 표식 쓰기가 갈아끼운다)"
        ),
        StateRead::Usable { file, warnings, .. } => {
            warnings
                .iter()
                .for_each(|warning| report_decode_warning(STATE_FILE, warning));
            if !file.clean_exit {
                tracing::warn!(
                    module = "state",
                    "앞 실행이 정상 종료를 적지 못했다 — 기본 화면으로 시작한다"
                );
            }
        }
    }
}

fn report_crash_copy_read(copy: &StateRead) {
    match copy {
        StateRead::Missing => {}
        StateRead::IoFailed(reason) => tracing::warn!(
            module = "state",
            reason = %reason,
            "state.crash.json 을 읽지 못했다 — 없는 것으로 치고 이번 부팅은 사본을 건드리지 않는다"
        ),
        StateRead::Unusable(unusable @ Unusable::NewerVersion { .. }) => tracing::warn!(
            module = "state",
            reason = %unusable,
            "state.crash.json 은 새 판이 쓴 사본이라 이 판이 적용하지 못한다 — 묻지 않고 그대로 둔다"
        ),
        StateRead::Unusable(unusable) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.crash.json 을 못 쓴다 — 그 사본으로는 묻지 않는다"
        ),
        StateRead::Usable { warnings, .. } => warnings
            .iter()
            .for_each(|warning| report_decode_warning(CRASH_COPY_FILE, warning)),
    }
}

fn report_decode_warning(file: &str, warning: &DecodeWarning) {
    match warning {
        DecodeWarning::WindowSkipped { index, id, reason } => tracing::warn!(
            module = "state",
            file,
            window_index = index,
            window_id = ?id,
            reason = %reason,
            "상태 파일의 창 하나를 건너뛴다"
        ),
        DecodeWarning::TabSkipped {
            window_id,
            index,
            reason,
        } => tracing::warn!(
            module = "state",
            file,
            window_id = %window_id,
            tab_index = index,
            reason = %reason,
            "상태 파일의 탭 하나를 건너뛴다"
        ),
        DecodeWarning::ThemeDropped { window_id, raw } => tracing::warn!(
            module = "state",
            file,
            window_id = %window_id,
            raw = %raw,
            "상태 파일의 창 테마를 모른다 — 전역 테마를 따른다"
        ),
    }
}

/// 부팅 단계 ④ — 동작의 결과 중 ⑤ · ⑥ · 기록기가 알아야 할 것(가드 ⅱ · 이어 실을 해시 · `state.json` 떠 두기)은
/// `plan` 에 접어 넣는다.
fn run_actions(files: &impl BootFiles, plan: &mut BootPlan) {
    let mut crash_copy_kept_aside = true;
    let replaces_crash_copy = plan.actions.contains(&BootAction::WriteCrashCopy);
    // 복사본을 돈다 — 동작마다 `plan` 의 다른 칸을 고칠 수 있어야 한다.
    for action in plan.actions.clone() {
        match action {
            BootAction::CopyAsideState => {
                let aside = match files.copy_aside_state() {
                    Ok(to) => {
                        tracing::info!(
                            module = "state",
                            to = %to.display(),
                            "못 쓰는 state.json 을 떠 뒀다"
                        );
                        StateAside::CopiedAside
                    }
                    Err(e) => {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "못 쓰는 state.json 을 떠 두지 못했다 — 진행한다: 실행 표식 쓰기가 하나뿐인 원본을 덮는다(D8)"
                        );
                        StateAside::NotCopied
                    }
                };
                plan.state_aside = Some(aside);
            }
            BootAction::CopyAsideCrashCopy => match files.copy_aside_crash_copy() {
                Ok(to) => tracing::info!(
                    module = "state",
                    to = %to.display(),
                    "못 쓰는 크래시 사본을 떠 뒀다"
                ),
                Err(e) => {
                    crash_copy_kept_aside = false;
                    if replaces_crash_copy {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "못 쓰는 크래시 사본을 떠 두지 못했다 — 진행한다: 곧 새 사본 쓰기가 그 원본을 백업 없이 갈아끼운다(D8 · ADR-0274 대가 2)"
                        );
                    } else {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "못 쓰는 크래시 사본을 떠 두지 못했다 — 지우지 않고 둔다: 다음 부팅이 다시 떠 둔다(D8)"
                        );
                    }
                }
            },
            BootAction::RemoveUnusableCrashCopy if !crash_copy_kept_aside => {}
            BootAction::RemoveUnusableCrashCopy => match files.remove_crash_copy() {
                Ok(()) => tracing::info!(module = "state", "못 쓰는 크래시 사본을 지웠다"),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    tracing::debug!(module = "state", "못 쓰는 크래시 사본이 이미 없다")
                }
                Err(e) => tracing::warn!(
                    module = "state",
                    error = %e,
                    "못 쓰는 크래시 사본을 못 지웠다 — 다음 부팅이 다시 지운다"
                ),
            },
            BootAction::WriteCrashCopy => write_crash_copy(files, plan),
            BootAction::RemoveAnsweredCrashCopy { hash } => {
                if let Some(hash) = remove_answered_copy(files, hash) {
                    plan.carry_resolved = Some(hash);
                }
            }
        }
    }
}

fn write_crash_copy(files: &impl BootFiles, plan: &mut BootPlan) {
    let Some(copy) = &plan.crash_copy else {
        // 판정은 이 동작과 뜰 원문을 늘 함께 싣는다 — 닿으면 판정의 결함이다.
        tracing::error!(
            module = "state",
            "크래시 사본을 뜰 원문이 계획에 없다 — 뜨지 못한 것으로 친다"
        );
        plan.guard = Some(Guard::CrashCopyNotWritten(
            "뜰 원문이 계획에 없다".to_string(),
        ));
        return;
    };
    match files.write_crash_copy(&copy.text) {
        Ok(()) => tracing::info!(
            module = "state",
            hash = %copy.hash,
            "앞 실행의 화면을 크래시 사본으로 떴다"
        ),
        Err(e) => {
            tracing::warn!(
                module = "state",
                error = %e,
                "크래시 사본을 뜨지 못했다 — 디스크의 state.json 을 그대로 둔다"
            );
            plan.guard = Some(Guard::CrashCopyNotWritten(e.to_string()));
        }
    }
}

/// N1 — 답한 사본을 다시 읽어 해시가 같을 때만 지운다. 돌려주는 값 = 이어 실을 해시(못 지웠다 · 다시 못 읽어
/// 답한 사본인지 모른다). 기록기의 같은 정리(`saver` 의 사본 지우기)와 같은 갈래다.
///
/// ★읽기와 지우기 사이는 원자적이지 않다★ — 그 틈을 막는 것은 셸 실행 잠금(I1)이다.
fn remove_answered_copy(files: &impl BootFiles, hash: String) -> Option<String> {
    match files.read_crash_copy() {
        Ok(text) if codec::crash_copy_hash(&text) == hash => match files.remove_crash_copy() {
            Ok(()) => {
                tracing::info!(module = "state", hash = %hash, "답한 크래시 사본을 지웠다");
                None
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                None
            }
            Err(e) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    error = %e,
                    "답한 크래시 사본을 못 지웠다 — 해시를 이어 싣고 기록기가 다시 지운다"
                );
                Some(hash)
            }
        },
        Ok(_) => {
            tracing::warn!(
                module = "state",
                hash = %hash,
                "크래시 사본이 판정 뒤 바뀌었다 — 새 사본이라 지우지 않는다"
            );
            None
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
            None
        }
        // 답한 사본은 상한 안의 UTF-8 이었다 — 그렇지 않은 사본은 다른 사본이다.
        Err(e) if codec::unusable_read(&e).is_some() => {
            tracing::warn!(
                module = "state",
                hash = %hash,
                error = %e,
                "크래시 사본이 판정 뒤 바뀌었다(못 쓰는 내용) — 지우지 않는다"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                module = "state",
                hash = %hash,
                error = %e,
                "크래시 사본을 다시 못 읽어 답한 사본인지 모른다 — 해시를 이어 싣고 기록기가 다시 본다"
            );
            Some(hash)
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

/// 부팅 단계 ⑤ — 실행 표식(`clean_exit:false`)과 판정한 모델의 창들 · 이어 실을 해시를 `state.json` 에 직접 쓴다.
/// 기록기는 아직 없다.
///
/// ★④ 뒤 · 어느 창보다 먼저 부른다★ — 사본 뜨기(④)가 끝난 뒤라야 원문을 덮어도 되고, 창보다 먼저라야 조용히
/// 복원한 화면이 창을 만들다 앱을 죽여도 다음 부팅이 비정상 종료로 읽는다(§6-5 ⑤). 실패는 log 만 하고 진행한다 —
/// 그 실행의 크래시는 정상 종료로 읽힐 수 있다(D8 · §12 R14).
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
        resolved_crash_copy: plan.carry_resolved.clone(),
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
    use std::collections::VecDeque;

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

    fn answered_state_file(clean_exit: bool, resolved: &str) -> StateFile {
        StateFile {
            resolved_crash_copy: Some(resolved.to_string()),
            ..state_file(clean_exit)
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

    fn text_of(file: &StateFile) -> String {
        codec::encode(file).unwrap()
    }

    fn read_of(file: StateFile) -> StateRead {
        StateRead::Usable {
            text: text_of(&file),
            file,
            warnings: Vec::new(),
        }
    }

    fn usable(clean_exit: bool) -> StateRead {
        read_of(state_file(clean_exit))
    }

    fn awaiting_of(file: StateFile) -> AwaitingCopy {
        let text = text_of(&file);
        AwaitingCopy {
            hash: codec::crash_copy_hash(&text),
            text,
            file,
        }
    }

    /// 앞앞 실행이 남긴 크래시 사본 — `state.json` 과 다른 바이트다.
    fn crash_file() -> StateFile {
        StateFile {
            saved_at_ms: 7,
            ..state_file(false)
        }
    }

    fn crash_text() -> String {
        text_of(&crash_file())
    }

    fn crash_hash() -> String {
        codec::crash_copy_hash(&crash_text())
    }

    fn newer_text() -> String {
        r#"{"version":2,"saved_at_ms":1,"clean_exit":false,"windows":[]}"#.to_string()
    }

    fn decide(state: StateRead) -> BootPlan {
        decide_boot(BootInputs {
            state,
            crash_copy: StateRead::Missing,
        })
    }

    fn quiet(model: BootModel) -> BootPlan {
        BootPlan {
            model,
            actions: Vec::new(),
            crash_copy: None,
            carry_resolved: None,
            guard: None,
            state_aside: None,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GuardKind {
        StateUnreadable,
        CrashCopyNotWritten,
    }

    fn guard_kind(guard: &Option<Guard>) -> Option<GuardKind> {
        guard.as_ref().map(|guard| match guard {
            Guard::StateUnreadable(_) => GuardKind::StateUnreadable,
            Guard::CrashCopyNotWritten(_) => GuardKind::CrashCopyNotWritten,
        })
    }

    // ── 판정표(§6-5) — 사본 열 × state.json 열 ──

    struct Row {
        name: &'static str,
        state: StateRead,
        crash_copy: StateRead,
        model: BootModel,
        actions: Vec<BootAction>,
        ask: Option<AwaitingCopy>,
        carry: Option<String>,
        guard: Option<GuardKind>,
    }

    impl Row {
        fn new(name: &'static str, state: StateRead, crash_copy: StateRead) -> Self {
            Self {
                name,
                state,
                crash_copy,
                model: BootModel::Default,
                actions: Vec::new(),
                ask: None,
                carry: None,
                guard: None,
            }
        }

        fn restores(self, file: StateFile) -> Self {
            Self {
                model: BootModel::Restore(file),
                ..self
            }
        }

        fn does(self, actions: Vec<BootAction>) -> Self {
            Self { actions, ..self }
        }

        fn asks(self, file: StateFile) -> Self {
            Self {
                ask: Some(awaiting_of(file)),
                ..self
            }
        }

        fn carries(self, hash: &str) -> Self {
            Self {
                carry: Some(hash.to_string()),
                ..self
            }
        }

        fn guarded(self, guard: GuardKind) -> Self {
            Self {
                guard: Some(guard),
                ..self
            }
        }
    }

    fn rows() -> Vec<Row> {
        use BootAction::*;
        use GuardKind::*;
        let crash = || read_of(crash_file());
        let newer = || StateRead::Unusable(Unusable::NewerVersion { found: 2 });
        let broken = || StateRead::Unusable(Unusable::NotJson("{".into()));
        let io = || StateRead::IoFailed("잠김".into());
        let missing = || StateRead::Missing;
        let answered = |clean| read_of(answered_state_file(clean, &crash_hash()));
        vec![
            // ── 있음 · 답 없음 → 기본 · 덮지 않음 · 묻는다(D2-6 · D6) ──
            Row::new("답 없는 사본 × state 없음", missing(), crash()).asks(crash_file()),
            Row::new("답 없는 사본 × clean_exit:true", usable(true), crash()).asks(crash_file()),
            Row::new("답 없는 사본 × clean_exit:false", usable(false), crash()).asks(crash_file()),
            Row::new(
                "답 없는 사본 × 다른 사본에 답한 state",
                read_of(answered_state_file(true, "옛 사본")),
                crash(),
            )
            .asks(crash_file()),
            Row::new("답 없는 사본 × 못 쓸 state(N6)", broken(), crash())
                .does(vec![CopyAsideState])
                .asks(crash_file()),
            Row::new("답 없는 사본 × 버전 초과 state", newer(), crash())
                .does(vec![CopyAsideState])
                .asks(crash_file()),
            Row::new("답 없는 사본 × state IO 실패(가드 ⅰ)", io(), crash())
                .asks(crash_file())
                .guarded(StateUnreadable),
            // ── 있음 · 버전 초과 → 없음으로 치고 · 손대지 않는다(N6) ──
            Row::new("버전 초과 사본 × state 없음", missing(), newer()),
            Row::new("버전 초과 사본 × clean_exit:true", usable(true), newer())
                .restores(state_file(true)),
            Row::new(
                "버전 초과 사본 × clean_exit:false(가드 ⅱ)",
                usable(false),
                newer(),
            )
            .asks(state_file(false))
            .guarded(CrashCopyNotWritten),
            Row::new("버전 초과 사본 × 못 쓸 state", broken(), newer()).does(vec![CopyAsideState]),
            Row::new("버전 초과 사본 × state IO 실패(가드 ⅰ)", io(), newer())
                .guarded(StateUnreadable),
            // ── 없음 ──
            Row::new("사본 없음 × state 없음", missing(), missing()),
            Row::new("사본 없음 × clean_exit:true", usable(true), missing())
                .restores(state_file(true)),
            Row::new(
                "사본 없음 × 답한 해시를 실은 clean_exit:true(이어 실을 해시가 없다)",
                read_of(answered_state_file(true, "h")),
                missing(),
            )
            .restores(answered_state_file(true, "h")),
            Row::new("사본 없음 × clean_exit:false(D5)", usable(false), missing())
                .does(vec![WriteCrashCopy])
                .asks(state_file(false)),
            Row::new("사본 없음 × 못 쓸 state", broken(), missing()).does(vec![CopyAsideState]),
            Row::new(
                "사본 없음 × 버전 초과 state(§12 R3)",
                StateRead::Unusable(Unusable::NewerVersion { found: 2 }),
                missing(),
            )
            .does(vec![CopyAsideState]),
            Row::new("사본 없음 × state IO 실패(가드 ⅰ)", io(), missing()).guarded(StateUnreadable),
            // ── 있음 · 답함 → 없음으로 치고 · 해시가 같을 때만 지운다(I2 · N1) ──
            Row::new("답한 사본 × clean_exit:true", answered(true), crash())
                .restores(answered_state_file(true, &crash_hash()))
                .does(vec![RemoveAnsweredCrashCopy { hash: crash_hash() }]),
            Row::new(
                "답한 사본 × clean_exit:false(새 사본이 갈아끼운다)",
                answered(false),
                crash(),
            )
            .does(vec![WriteCrashCopy])
            .asks(answered_state_file(false, &crash_hash())),
            // ── 못 쓸 사본 → 떠 둔 뒤 지운다(ADR-0274) ──
            Row::new("못 쓸 사본 × state 없음", missing(), broken())
                .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy]),
            Row::new("못 쓸 사본 × clean_exit:true", usable(true), broken())
                .restores(state_file(true))
                .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy]),
            Row::new(
                "못 쓸 사본 × clean_exit:false(떠 둔 뒤 새 사본)",
                usable(false),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, WriteCrashCopy])
            .asks(state_file(false)),
            Row::new(
                "못 쓸 사본(상한 초과) × 못 쓸 state",
                broken(),
                StateRead::Unusable(Unusable::Unreadable("상한 초과".into())),
            )
            .does(vec![
                CopyAsideState,
                CopyAsideCrashCopy,
                RemoveUnusableCrashCopy,
            ]),
            Row::new(
                "못 쓸 사본 × state IO 실패(가드 ⅰ — 사본 쪽은 사본 읽기만 본다)",
                io(),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy])
            .guarded(StateUnreadable),
            // ── 사본 읽기 IO 실패 → 없음으로 치고 · 사본 쪽 동작 0(I3 — 파일마다) ──
            Row::new("사본 IO 실패 × state 없음", missing(), io()),
            Row::new("사본 IO 실패 × clean_exit:true", usable(true), io())
                .restores(state_file(true)),
            Row::new(
                "사본 IO 실패 × 답한 해시를 실은 clean_exit:true(해시를 이어 싣는다)",
                read_of(answered_state_file(true, "h")),
                io(),
            )
            .restores(answered_state_file(true, "h"))
            .carries("h"),
            Row::new(
                "사본 IO 실패 × clean_exit:false(가드 ⅱ)",
                usable(false),
                io(),
            )
            .asks(state_file(false))
            .guarded(CrashCopyNotWritten),
            Row::new(
                "사본 IO 실패 × 답한 해시를 실은 clean_exit:false(가드 ⅱ — ⑤ 를 건너뛰어 해시가 디스크에 남는다)",
                read_of(answered_state_file(false, "h")),
                io(),
            )
            .asks(answered_state_file(false, "h"))
            .guarded(CrashCopyNotWritten),
            Row::new(
                "사본 IO 실패 × 못 쓸 state(state 쪽은 state 읽기만 본다 — 떠 둔다)",
                broken(),
                io(),
            )
            .does(vec![CopyAsideState]),
            Row::new(
                "사본 IO 실패 × 버전 초과 state(떠 둔다)",
                newer(),
                io(),
            )
            .does(vec![CopyAsideState]),
            Row::new("사본 IO 실패 × state IO 실패(가드 ⅰ)", io(), io()).guarded(StateUnreadable),
        ]
    }

    #[test]
    fn the_boot_table_decides_every_row() {
        let rows = rows();
        assert_eq!(rows.len(), 34, "행 수 — 행을 더하거나 빼면 이 수도 고친다");
        for row in rows {
            let plan = decide_boot(BootInputs {
                state: row.state,
                crash_copy: row.crash_copy,
            });
            assert_eq!(plan.model, row.model, "{} — 모델", row.name);
            assert_eq!(plan.actions, row.actions, "{} — 동작", row.name);
            assert_eq!(plan.crash_copy, row.ask, "{} — 묻나", row.name);
            assert_eq!(
                plan.carry_resolved, row.carry,
                "{} — 이어 실을 해시",
                row.name
            );
            assert_eq!(guard_kind(&plan.guard), row.guard, "{} — 가드", row.name);
            assert_eq!(
                plan.state_aside, None,
                "{} — 떠 두기 결과는 ④ 가 세운다",
                row.name
            );
        }
    }

    #[test]
    fn an_unusable_state_file_of_any_kind_is_copied_aside() {
        for unusable in [
            Unusable::Unreadable("상한 초과".into()),
            Unusable::NotJson("{".into()),
            Unusable::NotStateFile("머리 없음".into()),
            Unusable::NewerVersion { found: 2 },
        ] {
            assert_eq!(
                decide(StateRead::Unusable(unusable.clone())),
                BootPlan {
                    actions: vec![BootAction::CopyAsideState],
                    ..quiet(BootModel::Default)
                },
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn a_state_read_io_failure_carries_its_reason_in_the_guard() {
        assert_eq!(
            decide(StateRead::IoFailed("잠김".into())).guard,
            Some(Guard::StateUnreadable("잠김".into()))
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
            StateRead::from_read(Ok(newer_text())),
            StateRead::Unusable(Unusable::NewerVersion { found: 2 })
        );
        let text = text_of(&state_file(true));
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
        /// 사본 읽기마다 앞에서 하나씩 꺼낸다(② 의 읽기 · N1 의 다시 읽기) — 비면 `NotFound`.
        crash_reads: RefCell<VecDeque<io::Result<String>>>,
        copy_aside_fails: bool,
        write_fails: bool,
        sweep_fails: bool,
        crash_copy_aside_fails: bool,
        crash_write_fails: bool,
        crash_remove_fails: bool,
        calls: RefCell<Vec<String>>,
    }

    impl FakeFiles {
        fn reading(read: io::Result<String>) -> Self {
            Self {
                state: RefCell::new(Some(read)),
                ..Self::default()
            }
        }

        fn reading_both(state: io::Result<String>, crash: Vec<io::Result<String>>) -> Self {
            Self {
                crash_reads: RefCell::new(crash.into()),
                ..Self::reading(state)
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        /// 쓰기 호출의 글을 떼고 이름만.
        fn call_names(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .map(|call| match call.split_once(':') {
                    Some((name, _)) => name.to_string(),
                    None => call,
                })
                .collect()
        }

        fn written(&self) -> Vec<String> {
            self.texts("write:")
        }

        fn crash_written(&self) -> Vec<String> {
            self.texts("write_crash:")
        }

        fn texts(&self, prefix: &str) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter_map(|call| call.strip_prefix(prefix).map(str::to_string))
                .collect()
        }

        fn record(&self, call: String, fails: bool) -> io::Result<()> {
            let failure = fails.then(|| io::Error::other(format!("가짜 실패: {call}")));
            self.calls.borrow_mut().push(call);
            failure.map_or(Ok(()), Err)
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
            self.record("copy_aside".into(), self.copy_aside_fails)
                .map(|()| PathBuf::from("state.json.corrupt"))
        }

        fn write_state(&self, text: &str) -> io::Result<()> {
            self.record(format!("write:{text}"), self.write_fails)
        }

        fn read_crash_copy(&self) -> io::Result<String> {
            self.calls.borrow_mut().push("read_crash".into());
            self.crash_reads
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err(io::ErrorKind::NotFound.into()))
        }

        fn write_crash_copy(&self, text: &str) -> io::Result<()> {
            self.record(format!("write_crash:{text}"), self.crash_write_fails)
        }

        fn remove_crash_copy(&self) -> io::Result<()> {
            self.record("remove_crash".into(), self.crash_remove_fails)
        }

        fn copy_aside_crash_copy(&self) -> io::Result<PathBuf> {
            self.record("copy_aside_crash".into(), self.crash_copy_aside_fails)
                .map(|()| PathBuf::from("state.crash.json.corrupt"))
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

    fn marker_of(files: &FakeFiles) -> StateFile {
        let written = files.written();
        assert_eq!(written.len(), 1, "실행 표식은 한 번 쓴다");
        codec::decode(&written[0]).unwrap().0
    }

    #[test]
    fn an_unusable_state_file_is_copied_aside_once_after_the_sweep_and_the_reads() {
        let files = FakeFiles::reading(Ok("{".into()));

        let plan = prepare(&files);

        assert_eq!(plan.model, BootModel::Default);
        assert_eq!(plan.guard, None);
        assert_eq!(plan.state_aside, Some(StateAside::CopiedAside));
        assert_eq!(files.calls(), ["sweep", "read", "read_crash", "copy_aside"]);
    }

    #[test]
    fn a_failed_copy_aside_is_logged_and_the_boot_goes_on_to_write_the_marker() {
        let files = FakeFiles {
            copy_aside_fails: true,
            ..FakeFiles::reading(Ok("{".into()))
        };

        let plan = prepare(&files);

        assert_eq!(plan.guard, None, "떠 두기 실패는 가드가 아니다(D8)");
        assert_eq!(plan.state_aside, Some(StateAside::NotCopied));
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 7),
            MarkerOutcome::Written
        );
        assert_eq!(files.written().len(), 1);
    }

    #[test]
    fn a_state_read_io_failure_leaves_the_state_file_alone_and_writes_nothing_this_run() {
        let files = FakeFiles::reading_both(
            Err(io::ErrorKind::PermissionDenied.into()),
            vec![Ok("{".into())],
        );

        let plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::StateUnreadable(_))));
        assert_eq!(
            plan.state_aside, None,
            "못 쓸 사본을 떠 둔 것은 state.json 의 떠 두기가 아니다"
        );
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 7),
            MarkerOutcome::Guarded
        );
        assert_eq!(
            files.calls(),
            [
                "sweep",
                "read",
                "read_crash",
                "copy_aside_crash",
                "remove_crash"
            ],
            "state.json 은 떠 두기 · 쓰기 0회 — 못 쓸 사본은 사본 읽기만 보고 떠 둔 뒤 지운다(I3 는 파일마다)"
        );
    }

    #[test]
    fn a_clean_exit_restores_without_touching_the_disk_beyond_the_sweep_and_the_reads() {
        let files = FakeFiles::reading(Ok(text_of(&state_file(true))));

        let plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Restore(state_file(true))));
        assert_eq!(files.calls(), ["sweep", "read", "read_crash"]);
    }

    #[test]
    fn a_failed_sweep_does_not_stop_the_reads() {
        let files = FakeFiles {
            sweep_fails: true,
            ..FakeFiles::reading(Err(io::ErrorKind::NotFound.into()))
        };

        let plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Default));
        assert_eq!(files.calls(), ["sweep", "read", "read_crash"]);
    }

    // ── 사본 뜨기(D5) ──

    #[test]
    fn an_unclean_exit_copies_the_raw_state_bytes_before_the_marker_and_asks() {
        let raw = text_of(&state_file(false));
        let files = FakeFiles::reading(Ok(raw.clone()));

        let plan = prepare(&files);
        let marker = write_run_marker(&files, &plan, vec![tree_window()], 9);

        assert_eq!(plan.model, BootModel::Default);
        assert_eq!(plan.guard, None);
        assert_eq!(plan.crash_copy, Some(awaiting_of(state_file(false))));
        assert_eq!(
            files.crash_written(),
            [raw],
            "사본 = 읽은 원문 바이트 그대로"
        );
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(
            files.call_names(),
            ["sweep", "read", "read_crash", "write_crash", "write"],
            "사본 쓰기가 실행 표식보다 먼저"
        );
        let marker = marker_of(&files);
        assert!(!marker.clean_exit);
        assert_eq!(marker.resolved_crash_copy, None);
    }

    #[test]
    fn a_failed_crash_copy_write_guards_the_run_but_still_asks_from_memory() {
        let files = FakeFiles {
            crash_write_fails: true,
            ..FakeFiles::reading(Ok(text_of(&state_file(false))))
        };

        let plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::CrashCopyNotWritten(_))));
        assert_eq!(
            plan.crash_copy,
            Some(awaiting_of(state_file(false))),
            "복원 원천 = 메모리의 state.json 원문(L2)"
        );
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 9),
            MarkerOutcome::Guarded
        );
        assert!(files.written().is_empty(), "디스크의 state.json 그대로");
    }

    #[test]
    fn a_crash_copy_read_io_failure_with_an_unclean_exit_guards_without_writing() {
        let files = FakeFiles::reading_both(
            Ok(text_of(&state_file(false))),
            vec![Err(io::ErrorKind::PermissionDenied.into())],
        );

        let plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::CrashCopyNotWritten(_))));
        assert_eq!(plan.crash_copy, Some(awaiting_of(state_file(false))));
        assert_eq!(
            write_run_marker(&files, &plan, vec![tree_window()], 9),
            MarkerOutcome::Guarded
        );
        assert_eq!(files.calls(), ["sweep", "read", "read_crash"]);
    }

    #[test]
    fn a_crash_copy_read_io_failure_still_keeps_an_unusable_state_aside_before_the_marker() {
        for state in ["{".to_string(), newer_text()] {
            let files = FakeFiles::reading_both(
                Ok(state.clone()),
                vec![Err(io::ErrorKind::PermissionDenied.into())],
            );

            let plan = prepare(&files);
            let marker = write_run_marker(&files, &plan, vec![tree_window()], 9);

            assert_eq!(plan.guard, None, "{state}");
            assert_eq!(plan.state_aside, Some(StateAside::CopiedAside), "{state}");
            assert_eq!(marker, MarkerOutcome::Written, "{state}");
            assert_eq!(
                files.call_names(),
                ["sweep", "read", "read_crash", "copy_aside", "write"],
                "떠 두기가 실행 표식보다 먼저 · 사본 쪽 동작 0회(I3 는 파일마다) — {state}"
            );
        }
    }

    // ── 답 없는 사본 · 버전 초과 사본 ──

    #[test]
    fn an_unanswered_copy_is_never_overwritten_and_the_marker_still_goes_out() {
        let files =
            FakeFiles::reading_both(Ok(text_of(&state_file(false))), vec![Ok(crash_text())]);

        let plan = prepare(&files);
        let marker = write_run_marker(&files, &plan, vec![tree_window()], 9);

        assert_eq!(plan.crash_copy, Some(awaiting_of(crash_file())));
        assert!(files.crash_written().is_empty(), "사본을 덮지 않는다(D2-6)");
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(files.call_names(), ["sweep", "read", "read_crash", "write"]);
    }

    #[test]
    fn a_newer_version_copy_is_left_untouched() {
        for state in [state_file(true), state_file(false)] {
            let files = FakeFiles::reading_both(Ok(text_of(&state)), vec![Ok(newer_text())]);

            let plan = prepare(&files);

            assert_eq!(
                files.calls(),
                ["sweep", "read", "read_crash"],
                "지우지도 떠 두지도 덮지도 않는다(N6) — clean_exit:{}",
                state.clean_exit
            );
            assert_eq!(
                plan.guard.is_some(),
                !state.clean_exit,
                "떠야 할 사본을 못 뜨면 가드 ⅱ"
            );
        }
    }

    // ── 못 쓸 사본 ──

    #[test]
    fn an_unusable_copy_is_copied_aside_then_removed() {
        let files =
            FakeFiles::reading_both(Err(io::ErrorKind::NotFound.into()), vec![Ok("{".into())]);

        let plan = prepare(&files);

        assert_eq!(plan.crash_copy, None, "그 사본으로는 묻지 않는다");
        assert_eq!(
            files.calls(),
            [
                "sweep",
                "read",
                "read_crash",
                "copy_aside_crash",
                "remove_crash"
            ]
        );
    }

    #[test]
    fn an_unusable_copy_that_could_not_be_copied_aside_is_kept() {
        let files = FakeFiles {
            crash_copy_aside_fails: true,
            ..FakeFiles::reading_both(Err(io::ErrorKind::NotFound.into()), vec![Ok("{".into())])
        };

        let plan = prepare(&files);

        assert_eq!(plan.guard, None);
        assert_eq!(
            plan.state_aside, None,
            "사본의 떠 두기 실패는 state.json 의 것이 아니다"
        );
        assert_eq!(
            files.calls(),
            ["sweep", "read", "read_crash", "copy_aside_crash"],
            "둘 곳이 없는 원본은 지우지 않는다"
        );
    }

    // ── state.json 떠 두기 결과 — 사본 쪽 떠 두기와 섞지 않는다 ──

    #[test]
    fn the_state_aside_records_only_the_state_file_copy() {
        // (state 떠 두기 실패, 사본 떠 두기 실패) → 기대
        for (state_fails, crash_fails, expected) in [
            (false, false, StateAside::CopiedAside),
            (false, true, StateAside::CopiedAside),
            (true, false, StateAside::NotCopied),
            (true, true, StateAside::NotCopied),
        ] {
            let files = FakeFiles {
                copy_aside_fails: state_fails,
                crash_copy_aside_fails: crash_fails,
                ..FakeFiles::reading_both(Ok("{".into()), vec![Ok("{".into())])
            };

            let plan = prepare(&files);

            assert_eq!(
                plan.state_aside,
                Some(expected),
                "state 실패 {state_fails} · 사본 실패 {crash_fails}"
            );
            assert_eq!(plan.guard, None);
        }
    }

    #[test]
    fn an_unusable_state_with_an_unanswered_copy_records_the_aside_and_still_asks() {
        for (fails, expected) in [
            (false, StateAside::CopiedAside),
            (true, StateAside::NotCopied),
        ] {
            let files = FakeFiles {
                copy_aside_fails: fails,
                ..FakeFiles::reading_both(Ok("{".into()), vec![Ok(crash_text())])
            };

            let plan = prepare(&files);

            assert_eq!(plan.state_aside, Some(expected), "실패 {fails}");
            assert_eq!(plan.crash_copy, Some(awaiting_of(crash_file())));
            assert_eq!(plan.guard, None);
        }
    }

    #[test]
    fn a_usable_or_missing_state_records_no_aside() {
        for state in [
            Ok(text_of(&state_file(true))),
            Ok(text_of(&state_file(false))),
            Err(io::ErrorKind::NotFound.into()),
        ] {
            let plan = prepare(&FakeFiles::reading(state));
            assert_eq!(plan.state_aside, None);
        }
    }

    #[test]
    fn an_unusable_copy_is_replaced_by_the_new_copy_after_being_copied_aside() {
        let raw = text_of(&state_file(false));
        let files = FakeFiles::reading_both(Ok(raw.clone()), vec![Ok("{".into())]);

        let plan = prepare(&files);

        assert_eq!(
            files.call_names(),
            [
                "sweep",
                "read",
                "read_crash",
                "copy_aside_crash",
                "write_crash"
            ]
        );
        assert_eq!(files.crash_written(), [raw]);
        assert_eq!(plan.crash_copy, Some(awaiting_of(state_file(false))));
    }

    // ── 답한 사본(N1) ──

    fn answered_boot(crash_reads: Vec<io::Result<String>>) -> FakeFiles {
        FakeFiles::reading_both(
            Ok(text_of(&answered_state_file(true, &crash_hash()))),
            crash_reads,
        )
    }

    #[test]
    fn an_answered_copy_is_removed_after_a_matching_reread_and_nothing_is_carried() {
        let files = answered_boot(vec![Ok(crash_text()), Ok(crash_text())]);

        let plan = prepare(&files);
        write_run_marker(&files, &plan, vec![tree_window()], 9);

        assert_eq!(
            files.call_names(),
            [
                "sweep",
                "read",
                "read_crash",
                "read_crash",
                "remove_crash",
                "write"
            ]
        );
        assert_eq!(plan.carry_resolved, None);
        assert_eq!(plan.crash_copy, None, "그 사본으로는 묻지 않는다");
        assert_eq!(marker_of(&files).resolved_crash_copy, None);
    }

    #[test]
    fn an_answered_copy_that_changed_before_the_delete_is_not_removed() {
        let mut changed = crash_file();
        changed.saved_at_ms += 1;
        let files = answered_boot(vec![Ok(crash_text()), Ok(text_of(&changed))]);

        let plan = prepare(&files);

        assert_eq!(files.calls(), ["sweep", "read", "read_crash", "read_crash"]);
        assert_eq!(
            plan.carry_resolved, None,
            "다른 사본 — 기록기도 지우면 안 된다"
        );
    }

    #[test]
    fn an_answered_copy_that_became_unusable_before_the_delete_is_not_removed() {
        let files = answered_boot(vec![
            Ok(crash_text()),
            Err(io::Error::new(io::ErrorKind::InvalidData, "상한 초과")),
        ]);

        let plan = prepare(&files);

        assert_eq!(files.calls(), ["sweep", "read", "read_crash", "read_crash"]);
        assert_eq!(plan.carry_resolved, None);
    }

    #[test]
    fn an_answered_copy_gone_before_the_delete_carries_nothing() {
        let files = answered_boot(vec![Ok(crash_text())]);

        let plan = prepare(&files);

        assert_eq!(files.calls(), ["sweep", "read", "read_crash", "read_crash"]);
        assert_eq!(plan.carry_resolved, None);
    }

    #[test]
    fn a_failed_delete_carries_the_hash_into_the_marker() {
        let files = FakeFiles {
            crash_remove_fails: true,
            ..answered_boot(vec![Ok(crash_text()), Ok(crash_text())])
        };

        let plan = prepare(&files);
        let marker = write_run_marker(&files, &plan, vec![tree_window()], 9);

        assert_eq!(plan.carry_resolved, Some(crash_hash()));
        assert_eq!(plan.guard, None, "못 지운 것은 가드가 아니다");
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(
            marker_of(&files).resolved_crash_copy,
            Some(crash_hash()),
            "답이 디스크에 남는다(I2)"
        );
    }

    #[test]
    fn a_failed_reread_carries_the_hash() {
        let files = answered_boot(vec![
            Ok(crash_text()),
            Err(io::ErrorKind::PermissionDenied.into()),
        ]);

        let plan = prepare(&files);

        assert_eq!(files.calls(), ["sweep", "read", "read_crash", "read_crash"]);
        assert_eq!(plan.carry_resolved, Some(crash_hash()));
    }

    #[test]
    fn an_answered_copy_with_an_unclean_exit_is_replaced_by_the_new_copy() {
        let state = answered_state_file(false, &crash_hash());
        let files = FakeFiles::reading_both(Ok(text_of(&state)), vec![Ok(crash_text())]);

        let plan = prepare(&files);
        write_run_marker(&files, &plan, vec![tree_window()], 9);

        assert_eq!(
            files.call_names(),
            ["sweep", "read", "read_crash", "write_crash", "write"],
            "지우지 않고 새 사본 쓰기가 갈아끼운다"
        );
        assert_eq!(plan.carry_resolved, None);
        assert_eq!(marker_of(&files).resolved_crash_copy, None);
    }

    // ── 실행 표식(⑤) ──

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
    fn the_run_marker_carries_the_hash_to_carry() {
        let files = FakeFiles::default();
        let plan = BootPlan {
            carry_resolved: Some("h".into()),
            ..quiet(BootModel::Default)
        };

        write_run_marker(&files, &plan, Vec::new(), 7);

        assert_eq!(marker_of(&files).resolved_crash_copy, Some("h".into()));
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

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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

        assert_eq!(plan.state_aside, Some(StateAside::CopiedAside));
        assert_eq!(
            std::fs::read_to_string(dir.join("state.json.corrupt")).unwrap(),
            "{broken"
        );
        let text = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(codec::decode(&text).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unclean_state_file_on_disk_becomes_the_crash_copy_byte_for_byte() {
        let dir = temp_dir("unclean");
        std::fs::create_dir_all(&dir).unwrap();
        // BOM · CRLF 를 섞는다 — 다시 직렬화해 뜨면 이 바이트가 사라진다.
        let raw = format!(
            "\u{feff}{}",
            text_of(&state_file(false)).replace('\n', "\r\n")
        );
        std::fs::write(dir.join(STATE_FILE), &raw).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let plan = prepare(&files);
        write_run_marker(&files, &plan, Vec::new(), 7);

        assert_eq!(
            std::fs::read_to_string(dir.join(CRASH_COPY_FILE)).unwrap(),
            raw
        );
        assert_eq!(
            plan.crash_copy.as_ref().map(|copy| copy.text.as_str()),
            Some(raw.as_str())
        );
        assert_ne!(
            std::fs::read_to_string(dir.join(STATE_FILE)).unwrap(),
            raw,
            "실행 표식이 갈아끼웠다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_answered_copy_on_disk_is_removed_and_the_clean_state_restored() {
        let dir = temp_dir("answered");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_COPY_FILE), crash_text()).unwrap();
        let state = answered_state_file(true, &crash_hash());
        std::fs::write(dir.join(STATE_FILE), text_of(&state)).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let plan = prepare(&files);

        assert_eq!(plan.model, BootModel::Restore(state));
        assert_eq!(plan.carry_resolved, None);
        assert_eq!(names_in(&dir), [STATE_FILE]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unusable_copy_on_disk_is_kept_aside_and_removed() {
        let dir = temp_dir("broken-copy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_COPY_FILE), "{broken copy").unwrap();
        let files = FsBootFiles::in_dir(&dir);

        prepare(&files);

        assert_eq!(names_in(&dir), ["state.crash.json.corrupt"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("state.crash.json.corrupt")).unwrap(),
            "{broken copy"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_newer_copy_on_disk_keeps_its_bytes_and_gets_no_corrupt_twin() {
        let dir = temp_dir("newer-copy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CRASH_COPY_FILE), newer_text()).unwrap();
        std::fs::write(dir.join(STATE_FILE), text_of(&state_file(false))).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let plan = prepare(&files);
        write_run_marker(&files, &plan, Vec::new(), 7);

        assert_eq!(
            std::fs::read_to_string(dir.join(CRASH_COPY_FILE)).unwrap(),
            newer_text()
        );
        assert_eq!(names_in(&dir), [CRASH_COPY_FILE, STATE_FILE]);
        assert_eq!(
            std::fs::read_to_string(dir.join(STATE_FILE)).unwrap(),
            text_of(&state_file(false)),
            "가드 ⅱ — state.json 그대로"
        );
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

        assert_eq!(names_in(&dir), others);
        std::fs::remove_dir_all(&dir).ok();
    }
}
