//! Boot decision — ②–④ of the boot stage (TRD S21-storage §6-5) ([`prepare`]) and the run marker write of ⑤ ([`write_run_marker`]).
//! The decision ([`decide_boot`]) is a pure function, and all of the disk is behind the port [`BootFiles`] (production = [`FsBootFiles`]).
//!
//! - ★It continues whichever stage fails★ — the boot stage does not return `Err` (N7 · D8). Failures are logged here.
//! - ★The only exception is one guard ([`Guard`] · N3)★ — that run saves no state at all. Overwriting would lose a crash screen that may not have been
//!   copied aside yet.
//! - ★An unanswered crash copy is neither overwritten nor deleted (D2-6)★ — including the copy of a newer version that this version cannot use (N6). What boot deletes
//!   are only an unusable copy (after copying aside) and an answered copy (only when re-read and the hash is the same — N1).
//! - The result of ④ (the guard that stands because a copy could not be made · the hash of an answered copy that could not be deleted · whether an unusable `state.json` was copied aside) is
//!   folded into the plan by ④, and ⑤ ([`write_run_marker`]) · ⑥ · the writer read that plan — so the guard log is emitted after ④. If the re-decision just before ⑤'s
//!   write rejects, ⑤ folds the guard into the same field (⑥ reads the plan after ⑤).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use engram_dashboard_base::file;

use super::codec::{self, DecodeWarning, Unusable, STATE_READ_CAP};
use super::schema::{StateFile, WindowEntry, STATE_VERSION};
use crate::file_hooks::OS_HOOKS;

pub const STATE_FILE: &str = "state.json";
pub const CRASH_COPY_FILE: &str = "state.crash.json";

/// Files the boot stage touches — production is [`FsBootFiles`].
pub trait BootFiles {
    /// `state.json` text — same contract as `file::read_file_capped` (`NotFound` if missing · over the upper bound · not UTF-8 is
    /// `InvalidData` · a lock gives that error after retries).
    fn read_state(&self) -> io::Result<String>;

    /// Copies `state.json` aside as `state.json.corrupt` — returned value = the copy's place (`file::copy_aside`).
    fn copy_aside_state(&self) -> io::Result<PathBuf>;

    /// Atomically swaps `state.json` — creates the folder if it is missing.
    fn write_state(&self, text: &str) -> io::Result<()>;

    /// `state.crash.json` text — same contract as [`Self::read_state`].
    fn read_crash_copy(&self) -> io::Result<String>;

    /// Atomically swaps `state.crash.json` with `text` — creates the folder if it is missing.
    fn write_crash_copy(&self, text: &str) -> io::Result<()>;

    /// `NotFound` if missing.
    fn remove_crash_copy(&self) -> io::Result<()>;

    /// Copies `state.crash.json` aside as `state.crash.json.corrupt` — same contract as [`Self::copy_aside_state`].
    fn copy_aside_crash_copy(&self) -> io::Result<PathBuf>;

    /// Sweeps leftover temporary files of the state files — same contract as `file::sweep_temps`.
    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>>;
}

/// Production files — `<dir>\state.json` and its neighbors. `dir` = the shell state folder (layout in P3b3 · `DataLayout`).
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
        file::read_file_capped(&self.state(), STATE_READ_CAP, OS_HOOKS)
    }

    fn copy_aside_state(&self) -> io::Result<PathBuf> {
        file::copy_aside(&self.state(), OS_HOOKS)
    }

    // Nobody creates the shell state folder in advance — the writer creates it when writing (`file::write_atomic` · the recorder is the same).
    fn write_state(&self, text: &str) -> io::Result<()> {
        file::write_atomic(&self.state(), text.as_bytes(), OS_HOOKS)
    }

    fn read_crash_copy(&self) -> io::Result<String> {
        file::read_file_capped(&self.crash_copy(), STATE_READ_CAP, OS_HOOKS)
    }

    fn write_crash_copy(&self, text: &str) -> io::Result<()> {
        file::write_atomic(&self.crash_copy(), text.as_bytes(), OS_HOOKS)
    }

    // No lock retries — an answered copy that could not be deleted gets its hash picked up by the recorder and is deleted again, and an unusable copy is deleted again
    //   at the next boot.
    fn remove_crash_copy(&self) -> io::Result<()> {
        std::fs::remove_file(self.crash_copy())
    }

    fn copy_aside_crash_copy(&self) -> io::Result<PathBuf> {
        file::copy_aside(&self.crash_copy(), OS_HOOKS)
    }

    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
        file::sweep_temps(
            &self.dir,
            &[STATE_FILE, CRASH_COPY_FILE],
            engram_dashboard_platform::process::pid_alive,
        )
    }
}

/// The result of reading one state file (`state.json` or a crash copy of the same shape).
#[derive(Clone, PartialEq)]
pub enum StateRead {
    Missing,
    /// It is not the content but the read itself that failed (after lock retries · permission) — ★not mixed with an unusable file★: a healthy
    /// file that is briefly locked is neither copied aside nor overwritten (I3).
    IoFailed(String),
    Unusable(Unusable),
    /// `text` = the text read as it is — the source of copying aside · the copy hash · acceptance is these bytes (the disk is not read again —
    /// §6-5 ④ · L2).
    Usable {
        file: StateFile,
        text: String,
        warnings: Vec<DecodeWarning>,
    },
}

impl StateRead {
    /// Decides one read result (the contract of `file::read_file_capped`) — the sorting is one base decision (`file::classify` ·
    /// [`codec::STATE_SPEC`]).
    // ADR-0291
    pub fn from_read(read: io::Result<String>) -> Self {
        // The text is the source of copying aside · hash · acceptance, so it decides while holding it — what `classify(Ok(text))` does is this `parse`.
        let (parsed, text) = match read {
            Ok(text) => (file::parse(&text, &codec::STATE_SPEC), text),
            Err(e) => match file::classify(Err(e), &codec::STATE_SPEC) {
                file::Loaded::Missing => return StateRead::Missing,
                file::Loaded::Failed(e) => return StateRead::IoFailed(e.to_string()),
                // The only decisions that come from a read error are an unusable text (over the upper bound · not UTF-8), so the text field is not needed.
                file::Loaded::Parsed(parsed) => (parsed, String::new()),
            },
        };
        match codec::decode_parsed(parsed) {
            Ok((file, warnings)) => StateRead::Usable {
                file,
                text,
                warnings,
            },
            Err(unusable) => StateRead::Unusable(unusable),
        }
    }
}

// Does not print the text (≤4 MiB) — only the length.
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

/// Input of [`decide_boot`] — everything boot stage ② saw.
#[derive(Debug, Clone, PartialEq)]
pub struct BootInputs {
    pub state: StateRead,
    pub crash_copy: StateRead,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BootModel {
    /// Default layout (ADR-0222) — there is no state to restore, or it asks.
    Default,
    /// Restores quietly (「항상 복원」).
    Restore(StateFile),
}

/// The text that asks whether to restore — ⑥ hands it to the restore service. If there is an unanswered copy, that copy; if it is the copy this boot makes, the
/// `state.json` text — even if copying fails (guard ⅱ) the same text is used from memory (L2).
#[derive(Clone, PartialEq)]
pub struct AwaitingCopy {
    pub text: String,
    /// `codec::crash_copy_hash(&text)`.
    pub hash: String,
    pub file: StateFile,
}

// Does not print the text (≤4 MiB) — only the length and hash.
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
    /// Copies an unusable `state.json` (corrupt — not a newer version) aside as `state.json.corrupt` (ADR-0274) — the original is swapped by the run marker
    /// write (⑤). Proceeds even on failure (D8).
    CopyAsideState,
    /// Copies an unusable copy (not a version beyond this one) aside as `state.crash.json.corrupt` (ADR-0274). Proceeds even on failure (D8).
    CopyAsideCrashCopy,
    /// Deletes the unusable copy that was copied aside — ★if copying aside failed, it does not delete★: leave the original that has nowhere to go so that the next boot
    /// copies it aside again.
    RemoveUnusableCrashCopy,
    /// Copies the `state.json` text ([`BootPlan::crash_copy`]) as a copy — this write swaps any existing copy (answered · unusable).
    /// Failure = guard ⅱ.
    WriteCrashCopy,
    /// Re-reads the answered copy (≤4 MiB) and deletes it only when the hash is the same (N1). If it could not delete, or does not know whether it is an answered copy, passes on the hash
    /// ([`BootPlan::carry_resolved`]).
    RemoveAnsweredCrashCopy { hash: String },
}

/// This run saves no state at all (N3) — skips the run marker write (⑤) and does not start the recorder, and shutdown only releases the lock
/// without `Final` (N7). The next boot decides again (§12 R16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// ⅰ Could not read `state.json` (IO — after lock retries). One line of reason.
    StateUnreadable(String),
    /// The same field as ⅰ — `state.json` was written by a version newer than this shell (that version). Neither copied aside, overwritten nor deleted.
    // ADR-0291 R2: if copied aside and overwritten, the newer version loses its own screen state when it comes up again (nobody reads `.corrupt`).
    StateNewer { found: u64 },
    /// ⅱ Could not make the copy that had to be made — write failure · copy read IO failure · a copy of a newer version that must not be overwritten (N6). One line of reason.
    CrashCopyNotWritten(String),
}

/// The result of [`BootAction::CopyAsideState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateAside {
    CopiedAside,
    /// The original is swapped without a backup by the run marker write (⑤) (D8).
    NotCopied,
}

#[derive(Clone, PartialEq)]
pub struct BootPlan {
    pub model: BootModel,
    /// Boot stage ④ does this in this order.
    pub actions: Vec<BootAction>,
    /// `Some` = asks (`awaiting` of ⑥). It stands even with guard ⅱ.
    pub crash_copy: Option<AwaitingCopy>,
    /// The hash of an answered copy that ⑤ and the recorder carry on — it stands only when that copy may remain on the disk (decision · ④).
    pub carry_resolved: Option<String>,
    /// Set by the decision (③) or an action (④) — ⑤ looks at the value after ④, and if the re-decision just before the write rejects, ⑤ sets it too.
    pub guard: Option<Guard>,
    /// `Some` = `state.json` was an unusable file — the result of copying aside. Set by an action (④) (after the decision it is still `None`).
    pub state_aside: Option<StateAside>,
    /// `Adopted` if the `state.json` the decision (③) read was usable, `NotYet` otherwise — the recorder's start value is
    /// decided by [`saver_claim`] adding ⑤'s result to this.
    pub claim: file::Claim,
}

// Does not print the whole model to restore (a tab tree as large as the text ≤4 MiB) — only the window count.
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
            .field("claim", &self.claim)
            .finish()
    }
}

/// Copy column — apart from an unanswered copy it is counted as 「없음」 (none) and goes by the `state.json` row (§6-5 table).
enum CopyColumn {
    Unanswered(AwaitingCopy),
    AsAbsent(AbsentCopy),
}

/// What the copy counted as 「없음」 (none) is on the disk — the cleanup to do differs.
enum AbsentCopy {
    Missing,
    /// Read IO failure (I3).
    Unread,
    /// A copy written by a newer version — neither deleted, copied aside nor overwritten (N6).
    Newer,
    /// An unusable file other than a version beyond this one.
    Unusable,
    /// Hash = `resolved_crash_copy` of `state.json` (I2).
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

/// §6-5 table — copy column × `state.json` column.
pub fn decide_boot(inputs: BootInputs) -> BootPlan {
    let BootInputs { state, crash_copy } = inputs;
    let mut plan = BootPlan {
        model: BootModel::Default,
        actions: Vec::new(),
        crash_copy: None,
        carry_resolved: None,
        guard: None,
        state_aside: None,
        claim: match state {
            StateRead::Usable { .. } => file::Claim::Adopted,
            StateRead::Missing | StateRead::IoFailed(_) | StateRead::Unusable(_) => {
                file::Claim::NotYet
            }
        },
    };
    // I3 decides per file — only the file whose read had an IO failure is neither copied aside, overwritten nor deleted, and the action of the other file looks only at that file's
    //   read. If the two are bundled, a boot that could not read the copy overwrites an unusable `state.json` with ⑤ without copying it aside.

    // The fact that it was answered exists only in a readable `state.json` — if it cannot be read, the remaining copy is an unanswered copy.
    let resolved = match &state {
        StateRead::Usable { file, .. } => file.resolved_crash_copy.clone(),
        _ => None,
    };

    match &state {
        StateRead::IoFailed(reason) => plan.guard = Some(Guard::StateUnreadable(reason.clone())),
        // A file written by a newer version is in the same field as a read failure — neither copied aside nor overwritten, and this run does not save.
        // ADR-0291 R2
        StateRead::Unusable(Unusable::NewerVersion { found }) => {
            plan.guard = Some(Guard::StateNewer { found: *found })
        }
        // Copied aside regardless of the copy column (N6 · even if the copy could not be read — ⑤ swaps this original).
        StateRead::Unusable(Unusable::Corrupt(_)) => plan.actions.push(BootAction::CopyAsideState),
        StateRead::Missing | StateRead::Usable { .. } => {}
    }

    let copy = match CopyColumn::of(crash_copy, resolved.as_deref()) {
        // D2-6 · D4 — whatever `state.json` is, ask with the default screen and do not overwrite the copy.
        CopyColumn::Unanswered(awaiting) => {
            plan.crash_copy = Some(awaiting);
            return plan;
        }
        CopyColumn::AsAbsent(copy) => copy,
    };

    match state {
        StateRead::Usable { file, .. } if file.clean_exit => plan.model = BootModel::Restore(file),
        // D5 — copy the previous run's screen as a copy and ask (regardless of shape).
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
                // The answered copy is swapped by this write — there is no hash to carry on.
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
        // Cannot see whether the answered copy remains — carries the hash on. If dropped, the next boot would read that copy, which may still remain, as an
        //   unanswered copy and ask again.
        AbsentCopy::Unread => plan.carry_resolved = resolved,
        AbsentCopy::Missing | AbsentCopy::Newer => {}
    }
    plan
}

/// Boot stage ②–④ — sweeps leftover temporary files, reads and decides `state.json` and the crash copy, and does the action decided.
/// Failures are logged and it continues.
///
/// ★Call it after the shell run lock ([`super::lock::acquire`]) · before the recorder★ — only after the lock does it read the previous instance's last write,
/// and only before the recorder may it delete temporary files of its own pid.
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
        StateRead::Unusable(unusable @ Unusable::NewerVersion { .. }) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.json 은 이 셸보다 새 판이 쓴 것이다 — 기본 화면으로 시작하고 떠 두지도 덮지도 않는다(이번 실행은 화면 상태를 저장하지 않는다)"
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
        DecodeWarning::NumberDropped { window_id, raw } => tracing::warn!(
            module = "state",
            file,
            window_id = %window_id,
            raw = %raw,
            "상태 파일의 창 번호를 쓸 수 없다 — 버린다"
        ),
    }
}

/// Boot stage ④ — of the action results, what ⑤ · ⑥ · the recorder need to know (guard ⅱ · the hash to carry on · copying `state.json` aside) is
/// folded into `plan`.
fn run_actions(files: &impl BootFiles, plan: &mut BootPlan) {
    let mut crash_copy_kept_aside = true;
    let replaces_crash_copy = plan.actions.contains(&BootAction::WriteCrashCopy);
    // Iterates over a copy — each action must be able to edit a different field of `plan`.
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
        // The decision always carries this action together with the text to copy — reaching here is a defect of the decision.
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

/// N1 — re-reads the answered copy and deletes it only when the hash is the same. Returned value = the hash to carry on (could not delete · could not re-read, so does not know whether it is
/// an answered copy). It is the same branch as the recorder's same cleanup (copy deletion of `saver`).
///
/// ★Between reading and deleting is not atomic★ — what covers that gap is the shell run lock (I1).
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
        // The sorting of read errors is one base decision.
        Err(e) => match file::classify(Err(e), &codec::STATE_SPEC) {
            file::Loaded::Missing => {
                tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                None
            }
            // The answered copy was UTF-8 within the upper bound — a copy that is not is a different copy.
            file::Loaded::Parsed(parsed) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    parsed = ?parsed,
                    "크래시 사본이 판정 뒤 바뀌었다(못 쓰는 내용) — 지우지 않는다"
                );
                None
            }
            file::Loaded::Failed(e) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    error = %e,
                    "크래시 사본을 다시 못 읽어 답한 사본인지 모른다 — 해시를 이어 싣고 기록기가 다시 본다"
                );
                Some(hash)
            }
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerOutcome {
    Written,
    /// Being a guard, it did not write.
    Guarded,
    /// Codec rejection · disk error — logged.
    Failed,
}

/// Boot stage ⑤ — writes the run marker (`clean_exit:false`), the windows of the decided model and the hash to carry on directly into `state.json`.
/// The recorder does not exist yet.
///
/// ★Call it after ④ · before any window★ — only after copying aside (④) is finished may the original be overwritten, and only before windows so that even if a quietly
/// restored screen kills the app while making a window, the next boot reads it as an abnormal exit (§6-5 ⑤). Failures are only logged and it proceeds —
/// that run's crash may be read as a normal exit (D8 · §12 R14).
///
/// ★It decides the current `state.json` again just before writing★ — if rejected (newer version · could not read), it does not write, folds the same guard as guard ⅰ of ③ into
/// `plan` and gives `Guarded` (that run does not save, and `restore.status` of ⑥ carries that guard too).
pub fn write_run_marker(
    files: &impl BootFiles,
    plan: &mut BootPlan,
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
    if let Err(guard) = judge_before_marker(files, plan) {
        match &guard {
            Guard::StateNewer { found } => tracing::error!(
                module = "state",
                found,
                "실행 표식을 쓰기 직전의 state.json 이 이 셸보다 새 판이 쓴 것이다 — 덮지 않고 이번 실행은 화면 상태를 저장하지 않는다"
            ),
            _ => tracing::warn!(
                module = "state",
                guard = ?guard,
                "실행 표식을 쓰기 직전에 state.json 을 다시 못 읽었다 — 덮지 않고 이번 실행은 화면 상태를 저장하지 않는다"
            ),
        }
        plan.guard = Some(guard);
        return MarkerOutcome::Guarded;
    }
    match files.write_state(&text) {
        Ok(()) => MarkerOutcome::Written,
        Err(e) => {
            tracing::warn!(module = "state", error = %e, "실행 표식을 쓰지 못했다");
            MarkerOutcome::Failed
        }
    }
}

/// Re-decision just before the save of ⑤ — `Ok` if it may overwrite (if there is something to copy aside, after copying aside · `Ok` even if copying aside fails — D8), and if rejected, the guard of the same
/// field as guard ⅰ of ③.
// ADR-0291 R16: do not write holding the decision of ③ — do not overwrite a `state.json` that has changed into a newer version · an unreadable file since then.
fn judge_before_marker(files: &impl BootFiles, plan: &BootPlan) -> Result<(), Guard> {
    let current = file::classify(files.read_state(), &codec::STATE_SPEC);
    match current.write_policy(plan.claim) {
        file::WritePolicy::Write => Ok(()),
        // ④ has just copied this file aside (or failed while copying aside — D8) — it is within the same boot order, so it is not copied again.
        //   A file changed in between is an outside edit while running and is not supported (ADR-0291 C1).
        file::WritePolicy::CopyAsideFirst if plan.state_aside.is_some() => Ok(()),
        file::WritePolicy::CopyAsideFirst => {
            copy_state_aside_before_marker(files, &current);
            Ok(())
        }
        file::WritePolicy::Refuse(file::Refused::Newer { found }) => {
            Err(Guard::StateNewer { found })
        }
        file::WritePolicy::Refuse(file::Refused::ReadFailed) => {
            Err(Guard::StateUnreadable(match current {
                file::Loaded::Failed(e) => e.to_string(),
                // The decision table gives this rejection only for a read failure.
                _ => "state.json 을 못 읽었다".to_string(),
            }))
        }
    }
}

/// Proceeds even on failure — the run marker overwrites the only original (D8 · same as the recorder's same copying aside).
fn copy_state_aside_before_marker(files: &impl BootFiles, current: &file::Loaded) {
    let what = match current {
        file::Loaded::Parsed(file::Parsed::Usable { .. }) => "판정 뒤 놓인 state.json",
        _ => "판정 뒤 못 쓰게 된 state.json",
    };
    match files.copy_aside_state() {
        Ok(to) => tracing::info!(
            module = "state",
            to = %to.display(),
            "{what} 을 실행 표식으로 덮기 전에 떠 뒀다"
        ),
        Err(e) => tracing::error!(
            module = "state",
            "{what} 을 떠 두지 못했다 — 진행한다: 실행 표식 쓰기가 하나뿐인 원본을 덮는다(D8): {e}"
        ),
    }
}

/// The [`file::Claim`] the recorder starts holding — `Adopted` if the `state.json` the decision (③) read was usable or the run marker (⑤) was written.
/// Otherwise (missing · corrupt and ⑤ also could not write) it is `NotYet`, so a usable file placed in between is copied aside before the recorder's first
/// write overwrites it.
// ADR-0291 R8 (D14)
pub fn saver_claim(plan: &BootPlan, marker: MarkerOutcome) -> file::Claim {
    match marker {
        MarkerOutcome::Written => file::Claim::Adopted,
        MarkerOutcome::Guarded | MarkerOutcome::Failed => plan.claim,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;
    use crate::state::schema::{Bounds, TabStrip, WindowKind};

    fn state_file(clean_exit: bool) -> StateFile {
        StateFile {
            version: STATE_VERSION,
            saved_at_ms: 42,
            clean_exit,
            resolved_crash_copy: None,
            windows: vec![a_window()],
        }
    }

    fn answered_state_file(clean_exit: bool, resolved: &str) -> StateFile {
        StateFile {
            resolved_crash_copy: Some(resolved.to_string()),
            ..state_file(clean_exit)
        }
    }

    fn a_window() -> WindowEntry {
        WindowEntry {
            id: "main".into(),
            number: Some(1),
            kind: WindowKind::Main(TabStrip {
                active_tab: uuid::Uuid::nil(),
                tabs: Vec::new(),
            }),
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

    /// A crash copy left by the run before the previous one — bytes different from `state.json`.
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

    /// A plan with no action — a restoring plan comes only from a usable `state.json`, so it carries `Adopted`.
    fn quiet(model: BootModel) -> BootPlan {
        let claim = match model {
            BootModel::Restore(_) => file::Claim::Adopted,
            BootModel::Default => file::Claim::NotYet,
        };
        BootPlan {
            model,
            actions: Vec::new(),
            crash_copy: None,
            carry_resolved: None,
            guard: None,
            state_aside: None,
            claim,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GuardKind {
        StateUnreadable,
        StateNewer,
        CrashCopyNotWritten,
    }

    fn guard_kind(guard: &Option<Guard>) -> Option<GuardKind> {
        guard.as_ref().map(|guard| match guard {
            Guard::StateUnreadable(_) => GuardKind::StateUnreadable,
            Guard::StateNewer { .. } => GuardKind::StateNewer,
            Guard::CrashCopyNotWritten(_) => GuardKind::CrashCopyNotWritten,
        })
    }

    // ── decision table (§6-5) — copy column × state.json column ──

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
        let broken = || StateRead::Unusable(Unusable::Corrupt("{".into()));
        let io = || StateRead::IoFailed("잠김".into());
        let missing = || StateRead::Missing;
        let answered = |clean| read_of(answered_state_file(clean, &crash_hash()));
        vec![
            // ── present · unanswered → default · not overwritten · asks (D2-6 · D6) ──
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
            Row::new("답 없는 사본 × 버전 초과 state(가드 — 떠 두지 않는다)", newer(), crash())
                .asks(crash_file())
                .guarded(StateNewer),
            Row::new("답 없는 사본 × state IO 실패(가드 ⅰ)", io(), crash())
                .asks(crash_file())
                .guarded(StateUnreadable),
            // ── present · version beyond this one → counted as none · not touched (N6) ──
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
            Row::new("버전 초과 사본 × 버전 초과 state(가드)", newer(), newer()).guarded(StateNewer),
            // ── none ──
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
                "사본 없음 × 버전 초과 state(가드 — 떠 두지도 덮지도 않는다 · ADR-0291 R2)",
                newer(),
                missing(),
            )
            .guarded(StateNewer),
            Row::new("사본 없음 × state IO 실패(가드 ⅰ)", io(), missing()).guarded(StateUnreadable),
            // ── present · answered → counted as none · deleted only when the hash is the same (I2 · N1) ──
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
            // ── unusable copy → deleted after copying aside (ADR-0274) ──
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
                StateRead::Unusable(Unusable::Corrupt("상한 초과".into())),
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
            Row::new(
                "못 쓸 사본 × 버전 초과 state(가드 — 사본 쪽은 사본 읽기만 본다)",
                newer(),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy])
            .guarded(StateNewer),
            // ── copy read IO failure → counted as none · zero actions on the copy side (I3 — per file) ──
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
                "사본 IO 실패 × 버전 초과 state(가드 — 떠 두지 않는다)",
                newer(),
                io(),
            )
            .guarded(StateNewer),
            Row::new("사본 IO 실패 × state IO 실패(가드 ⅰ)", io(), io()).guarded(StateUnreadable),
        ]
    }

    #[test]
    fn the_boot_table_decides_every_row() {
        let rows = rows();
        assert_eq!(rows.len(), 36, "행 수 — 행을 더하거나 빼면 이 수도 고친다");
        for row in rows {
            let claim = match row.state {
                StateRead::Usable { .. } => file::Claim::Adopted,
                _ => file::Claim::NotYet,
            };
            let plan = decide_boot(BootInputs {
                state: row.state,
                crash_copy: row.crash_copy,
            });
            assert_eq!(plan.claim, claim, "{} — 기록기의 시작 판정 축", row.name);
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
    fn a_corrupt_state_file_of_any_kind_is_copied_aside() {
        for reason in ["상한 초과", "JSON 이 아니다", "상태 파일 모양이 아니다"] {
            assert_eq!(
                decide(StateRead::Unusable(Unusable::Corrupt(reason.into()))),
                BootPlan {
                    actions: vec![BootAction::CopyAsideState],
                    ..quiet(BootModel::Default)
                },
                "{reason}"
            );
        }
    }

    /// ADR-0291 R2: a `state.json` written by a newer version is not copied aside and is a guard — it carries that version.
    #[test]
    fn a_newer_state_file_guards_the_run_and_is_not_copied_aside() {
        assert_eq!(
            decide(StateRead::Unusable(Unusable::NewerVersion { found: 2 })),
            BootPlan {
                guard: Some(Guard::StateNewer { found: 2 }),
                ..quiet(BootModel::Default)
            }
        );
    }

    #[test]
    fn a_state_read_io_failure_carries_its_reason_in_the_guard() {
        assert_eq!(
            decide(StateRead::IoFailed("잠김".into())).guard,
            Some(Guard::StateUnreadable("잠김".into()))
        );
    }

    // ── sorting read results ──

    #[test]
    fn read_errors_split_into_missing_unusable_and_io_failure() {
        let from_err = |e: io::Error| StateRead::from_read(Err(e));
        assert_eq!(from_err(io::ErrorKind::NotFound.into()), StateRead::Missing);
        assert!(matches!(
            from_err(io::Error::new(io::ErrorKind::InvalidData, "상한 초과")),
            StateRead::Unusable(Unusable::Corrupt(_))
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
            StateRead::Unusable(Unusable::Corrupt(_))
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

    // ── boot stage ②–④ · ⑤ (fake files) ──

    #[derive(Default)]
    struct FakeFiles {
        state: RefCell<Option<io::Result<String>>>,
        /// Takes one from the front at every copy read (the read of ② · the re-read of N1) — `NotFound` when empty.
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

        /// Strips the text of the write call and keeps only the name.
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
        // Remains even when read like a disk — the read of ③ and the re-decision of ⑤ look at the same field, and the test changes the field in between.
        //   Empty field = no file.
        fn read_state(&self) -> io::Result<String> {
            self.calls.borrow_mut().push("read".into());
            match &*self.state.borrow() {
                None => Err(io::ErrorKind::NotFound.into()),
                Some(Ok(text)) => Ok(text.clone()),
                Some(Err(e)) => Err(io::Error::new(e.kind(), e.to_string())),
            }
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

        let mut plan = prepare(&files);

        assert_eq!(plan.guard, None, "떠 두기 실패는 가드가 아니다(D8)");
        assert_eq!(plan.state_aside, Some(StateAside::NotCopied));
        assert_eq!(
            write_run_marker(&files, &mut plan, vec![a_window()], 7),
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

        let mut plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::StateUnreadable(_))));
        assert_eq!(
            plan.state_aside, None,
            "못 쓸 사본을 떠 둔 것은 state.json 의 떠 두기가 아니다"
        );
        assert_eq!(
            write_run_marker(&files, &mut plan, vec![a_window()], 7),
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

    // ── copying a copy aside (D5) ──

    #[test]
    fn an_unclean_exit_copies_the_raw_state_bytes_before_the_marker_and_asks() {
        let raw = text_of(&state_file(false));
        let files = FakeFiles::reading(Ok(raw.clone()));

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

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
            [
                "sweep",
                "read",
                "read_crash",
                "write_crash",
                "read",
                "write"
            ],
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

        let mut plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::CrashCopyNotWritten(_))));
        assert_eq!(
            plan.crash_copy,
            Some(awaiting_of(state_file(false))),
            "복원 원천 = 메모리의 state.json 원문(L2)"
        );
        assert_eq!(
            write_run_marker(&files, &mut plan, vec![a_window()], 9),
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

        let mut plan = prepare(&files);

        assert!(matches!(plan.guard, Some(Guard::CrashCopyNotWritten(_))));
        assert_eq!(plan.crash_copy, Some(awaiting_of(state_file(false))));
        assert_eq!(
            write_run_marker(&files, &mut plan, vec![a_window()], 9),
            MarkerOutcome::Guarded
        );
        assert_eq!(files.calls(), ["sweep", "read", "read_crash"]);
    }

    #[test]
    fn a_crash_copy_read_io_failure_still_keeps_an_unusable_state_aside_before_the_marker() {
        let files = FakeFiles::reading_both(
            Ok("{".into()),
            vec![Err(io::ErrorKind::PermissionDenied.into())],
        );

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

        assert_eq!(plan.guard, None);
        assert_eq!(plan.state_aside, Some(StateAside::CopiedAside));
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(
            files.call_names(),
            ["sweep", "read", "read_crash", "copy_aside", "read", "write"],
            "떠 두기가 실행 표식보다 먼저 · ⑤ 의 재판정은 다시 떠 두지 않는다 · 사본 쪽 동작 0회(I3 는 파일마다)"
        );
    }

    /// ADR-0291 R2: a `state.json` written by a newer version is neither copied aside nor overwritten and this run does not save — the copy side
    /// looks only at the copy read (I3 is per file).
    #[test]
    fn a_newer_state_file_is_left_alone_and_the_run_saves_nothing() {
        let files = FakeFiles::reading_both(Ok(newer_text()), vec![Ok("{".into())]);

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

        assert_eq!(plan.guard, Some(Guard::StateNewer { found: 2 }));
        assert_eq!(plan.state_aside, None);
        assert_eq!(plan.model, BootModel::Default);
        assert_eq!(marker, MarkerOutcome::Guarded);
        assert_eq!(
            files.calls(),
            [
                "sweep",
                "read",
                "read_crash",
                "copy_aside_crash",
                "remove_crash"
            ],
            "state.json 은 떠 두기 · 쓰기 0회"
        );
    }

    /// ADR-0291 R16: ⑤ does not write holding the decision of ③ — if it becomes a newer version · an unreadable file in between, it is neither copied aside nor overwritten
    /// and carries into the plan the guard of the same field as guard ⅰ of ③ (this run does not save).
    #[test]
    fn the_marker_never_overwrites_a_state_file_that_turned_newer_or_unreadable() {
        for (now, guard) in [
            (Ok(newer_text()), Guard::StateNewer { found: 2 }),
            (
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "잠김")),
                Guard::StateUnreadable("잠김".into()),
            ),
        ] {
            let label = format!("{guard:?}");
            let files = FakeFiles::reading(Ok(text_of(&state_file(true))));
            let mut plan = prepare(&files);
            assert_eq!(plan.guard, None, "{label}");
            *files.state.borrow_mut() = Some(now);

            let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

            assert_eq!(marker, MarkerOutcome::Guarded, "{label}");
            assert_eq!(plan.guard, Some(guard), "{label}");
            assert_eq!(
                files.calls(),
                ["sweep", "read", "read_crash", "read"],
                "떠 두기 · 쓰기 0회 — {label}"
            );
        }
    }

    /// ADR-0291 R8 · R16: if a file that was not in ③ was placed before ⑤ (corrupt or usable — D14), it is written after copying aside. Even if copying aside
    /// fails, it writes (D8).
    #[test]
    fn a_state_file_placed_after_the_decision_is_copied_aside_before_the_marker() {
        for (placed, aside_fails) in [
            ("{".to_string(), false),
            (text_of(&state_file(true)), false),
            ("{".to_string(), true),
        ] {
            let files = FakeFiles {
                copy_aside_fails: aside_fails,
                ..FakeFiles::reading(Err(io::ErrorKind::NotFound.into()))
            };
            let mut plan = prepare(&files);
            *files.state.borrow_mut() = Some(Ok(placed.clone()));

            let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

            assert_eq!(marker, MarkerOutcome::Written, "{placed} · {aside_fails}");
            assert_eq!(plan.guard, None, "{placed} · {aside_fails}");
            assert_eq!(
                files.call_names(),
                ["sweep", "read", "read_crash", "read", "copy_aside", "write"],
                "{placed} · {aside_fails}"
            );
        }
    }

    #[test]
    fn the_saver_starts_adopted_when_the_state_was_usable_or_the_marker_went_out() {
        let usable = decide(usable(true));
        let missing = decide(StateRead::Missing);
        assert_eq!(
            saver_claim(&usable, MarkerOutcome::Failed),
            file::Claim::Adopted
        );
        assert_eq!(
            saver_claim(&missing, MarkerOutcome::Written),
            file::Claim::Adopted
        );
        assert_eq!(
            saver_claim(&missing, MarkerOutcome::Failed),
            file::Claim::NotYet,
            "없던 파일이고 ⑤ 도 못 썼다 — 그 사이 놓인 파일은 이 실행의 것이 아니다"
        );
        let corrupt = decide(StateRead::Unusable(Unusable::Corrupt("{".into())));
        assert_eq!(
            saver_claim(&corrupt, MarkerOutcome::Failed),
            file::Claim::NotYet
        );
    }

    // ── unanswered copy · copy of a version beyond this one ──

    #[test]
    fn an_unanswered_copy_is_never_overwritten_and_the_marker_still_goes_out() {
        let files =
            FakeFiles::reading_both(Ok(text_of(&state_file(false))), vec![Ok(crash_text())]);

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

        assert_eq!(plan.crash_copy, Some(awaiting_of(crash_file())));
        assert!(files.crash_written().is_empty(), "사본을 덮지 않는다(D2-6)");
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(
            files.call_names(),
            ["sweep", "read", "read_crash", "read", "write"]
        );
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

    // ── unusable copy ──

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

    // ── result of copying state.json aside — not mixed with the copy side's copying aside ──

    #[test]
    fn the_state_aside_records_only_the_state_file_copy() {
        // (state copy-aside failure, copy copy-aside failure) → expected
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

    // ── answered copy (N1) ──

    fn answered_boot(crash_reads: Vec<io::Result<String>>) -> FakeFiles {
        FakeFiles::reading_both(
            Ok(text_of(&answered_state_file(true, &crash_hash()))),
            crash_reads,
        )
    }

    #[test]
    fn an_answered_copy_is_removed_after_a_matching_reread_and_nothing_is_carried() {
        let files = answered_boot(vec![Ok(crash_text()), Ok(crash_text())]);

        let mut plan = prepare(&files);
        write_run_marker(&files, &mut plan, vec![a_window()], 9);

        assert_eq!(
            files.call_names(),
            [
                "sweep",
                "read",
                "read_crash",
                "read_crash",
                "remove_crash",
                "read",
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

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 9);

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

        let mut plan = prepare(&files);
        write_run_marker(&files, &mut plan, vec![a_window()], 9);

        assert_eq!(
            files.call_names(),
            [
                "sweep",
                "read",
                "read_crash",
                "write_crash",
                "read",
                "write"
            ],
            "지우지 않고 새 사본 쓰기가 갈아끼운다"
        );
        assert_eq!(plan.carry_resolved, None);
        assert_eq!(marker_of(&files).resolved_crash_copy, None);
    }

    // ── run marker (⑤) ──

    #[test]
    fn the_run_marker_is_an_unclean_exit_with_the_given_windows() {
        let files = FakeFiles::default();

        let outcome = write_run_marker(
            &files,
            &mut quiet(BootModel::Restore(state_file(true))),
            vec![a_window()],
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
                windows: vec![a_window()],
            }
        );
    }

    #[test]
    fn the_run_marker_carries_the_hash_to_carry() {
        let files = FakeFiles::default();
        let mut plan = BootPlan {
            carry_resolved: Some("h".into()),
            ..quiet(BootModel::Default)
        };

        write_run_marker(&files, &mut plan, Vec::new(), 7);

        assert_eq!(marker_of(&files).resolved_crash_copy, Some("h".into()));
    }

    #[test]
    fn a_failed_marker_write_is_reported_not_raised() {
        let files = FakeFiles {
            write_fails: true,
            ..FakeFiles::default()
        };

        let outcome = write_run_marker(&files, &mut quiet(BootModel::Default), Vec::new(), 7);

        assert_eq!(outcome, MarkerOutcome::Failed);
    }

    // ── production files (temporary folder) ──

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

        let mut plan = prepare(&files);

        assert_eq!(plan, quiet(BootModel::Default));
        assert_eq!(
            write_run_marker(&files, &mut plan, Vec::new(), 7),
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

        let mut plan = prepare(&files);
        write_run_marker(&files, &mut plan, Vec::new(), 7);

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
        // Mixes BOM · CRLF — if it were re-serialized and copied, these bytes would disappear.
        let raw = format!(
            "\u{feff}{}",
            text_of(&state_file(false)).replace('\n', "\r\n")
        );
        std::fs::write(dir.join(STATE_FILE), &raw).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let mut plan = prepare(&files);
        write_run_marker(&files, &mut plan, Vec::new(), 7);

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

        let mut plan = prepare(&files);
        write_run_marker(&files, &mut plan, Vec::new(), 7);

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
    fn a_newer_state_file_on_disk_keeps_its_bytes_and_gets_no_corrupt_twin() {
        let dir = temp_dir("newer-state");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STATE_FILE), newer_text()).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let mut plan = prepare(&files);
        let marker = write_run_marker(&files, &mut plan, Vec::new(), 7);

        assert_eq!(plan.guard, Some(Guard::StateNewer { found: 2 }));
        assert_eq!(marker, MarkerOutcome::Guarded);
        assert_eq!(
            std::fs::read_to_string(dir.join(STATE_FILE)).unwrap(),
            newer_text()
        );
        assert_eq!(names_in(&dir), [STATE_FILE]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// ADR-0291 R16: even a real file that changed into a newer version after ③ and before ⑤ stays with those bytes as they are.
    #[test]
    fn a_state_file_that_turns_newer_before_the_marker_keeps_its_bytes_on_disk() {
        let dir = temp_dir("newer-before-marker");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STATE_FILE), text_of(&state_file(true))).unwrap();
        let files = FsBootFiles::in_dir(&dir);

        let mut plan = prepare(&files);
        assert_eq!(plan.guard, None);
        std::fs::write(dir.join(STATE_FILE), newer_text()).unwrap();
        let marker = write_run_marker(&files, &mut plan, vec![a_window()], 7);

        assert_eq!(marker, MarkerOutcome::Guarded);
        assert_eq!(plan.guard, Some(Guard::StateNewer { found: 2 }));
        assert_eq!(
            std::fs::read_to_string(dir.join(STATE_FILE)).unwrap(),
            newer_text()
        );
        assert_eq!(names_in(&dir), [STATE_FILE]);
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
