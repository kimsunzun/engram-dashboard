//! Boot decision — ②–④ of the boot stage (TRD S21-storage §6-5, [`prepare`]) and the run marker write of ⑤ ([`write_run_marker`]). The decision ([`decide_boot`]) is a pure function; all disk access is behind the port [`BootFiles`] (production = [`FsBootFiles`]).
//! - Continues whichever stage fails — the boot stage never returns `Err` (N7 · D8); failures are logged here.
//! - The only exception is one guard ([`Guard`] · N3): that run saves no state at all, since overwriting could lose a crash screen not yet copied aside.
//! - An unanswered crash copy is neither overwritten nor deleted (D2-6), including a newer version's copy this version cannot use (N6). Boot deletes only an unusable copy (after copying aside) and an answered copy (only when re-read and the hash is the same — N1).
//! - ④'s results (the guard standing because a copy could not be made · the hash of an answered copy that could not be deleted · whether an unusable `state.json` was copied aside) are folded into the plan; ⑤ ([`write_run_marker`]) · ⑥ · the writer read that plan, so the guard log is emitted after ④. If the re-decision just before ⑤'s write rejects, ⑤ folds the guard into the same field (⑥ reads the plan after ⑤).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use engram_dashboard_base::file;

use super::codec::{self, DecodeWarning, Unusable, STATE_READ_CAP};
use super::schema::{StateFile, WindowEntry, STATE_VERSION};
use crate::file_hooks::OS_HOOKS;

pub const STATE_FILE: &str = "state.json";
pub const CRASH_COPY_FILE: &str = "state.crash.json";

pub trait BootFiles {
    /// `state.json` text — same contract as `file::read_file_capped` (`NotFound` if missing · over the upper bound · not UTF-8 is `InvalidData` · a lock gives that error after retries).
    fn read_state(&self) -> io::Result<String>;

    fn copy_aside_state(&self) -> io::Result<PathBuf>;

    fn write_state(&self, text: &str) -> io::Result<()>;

    /// `state.crash.json` text — same contract as [`Self::read_state`].
    fn read_crash_copy(&self) -> io::Result<String>;

    fn write_crash_copy(&self, text: &str) -> io::Result<()>;

    fn remove_crash_copy(&self) -> io::Result<()>;

    fn copy_aside_crash_copy(&self) -> io::Result<PathBuf>;

    /// Sweeps leftover temporary files of the state files — same contract as `file::sweep_temps`.
    fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>>;
}

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

    fn write_state(&self, text: &str) -> io::Result<()> {
        file::write_atomic(&self.state(), text.as_bytes(), OS_HOOKS)
    }

    fn read_crash_copy(&self) -> io::Result<String> {
        file::read_file_capped(&self.crash_copy(), STATE_READ_CAP, OS_HOOKS)
    }

    fn write_crash_copy(&self, text: &str) -> io::Result<()> {
        file::write_atomic(&self.crash_copy(), text.as_bytes(), OS_HOOKS)
    }

    // No lock retries: an answered copy that could not be deleted has its hash picked up by the recorder and deleted again; an unusable copy is deleted again at the next boot.
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

#[derive(Clone, PartialEq)]
pub enum StateRead {
    Missing,
    /// The read itself failed (after lock retries · permission), not the content — never mixed with an unusable file: a healthy but briefly locked file is neither copied aside nor overwritten (I3).
    IoFailed(String),
    Unusable(Unusable),
    /// `text` = the text as read — the source of copying aside · the copy hash · acceptance is these bytes (the disk is not read again — §6-5 ④ · L2).
    Usable {
        file: StateFile,
        text: String,
        warnings: Vec<DecodeWarning>,
    },
}

impl StateRead {
    /// Decides one read result (contract of `file::read_file_capped`) — the sorting is one base decision (`file::classify` · [`codec::STATE_SPEC`]).
    // ADR-0291
    pub fn from_read(read: io::Result<String>) -> Self {
        let (parsed, text) = match read {
            Ok(text) => (file::parse(&text, &codec::STATE_SPEC), text),
            Err(e) => match file::classify(Err(e), &codec::STATE_SPEC) {
                file::Loaded::Missing => return StateRead::Missing,
                file::Loaded::Failed(e) => return StateRead::IoFailed(e.to_string()),
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

#[derive(Debug, Clone, PartialEq)]
pub struct BootInputs {
    pub state: StateRead,
    pub crash_copy: StateRead,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BootModel {
    /// ADR-0222
    Default,
    Restore(StateFile),
}

/// The text that asks whether to restore, handed to the restore service by ⑥: an unanswered copy if there is one, else the `state.json` text this boot copies — held in memory even if copying fails (guard ⅱ; L2).
#[derive(Clone, PartialEq)]
pub struct AwaitingCopy {
    pub text: String,
    pub hash: String,
    pub file: StateFile,
}

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
    /// Copies an unusable `state.json` (corrupt — not a newer version) aside as `state.json.corrupt` (ADR-0274); the original is swapped by the run marker write (⑤). Proceeds even on failure (D8).
    CopyAsideState,
    /// ADR-0274
    CopyAsideCrashCopy,
    /// Deletes the unusable copy that was copied aside — if copying aside failed it does not delete, leaving the original with nowhere to go so the next boot copies it aside again.
    RemoveUnusableCrashCopy,
    /// Copies the `state.json` text ([`BootPlan::crash_copy`]) as a copy — this write swaps any existing copy (answered · unusable). Failure = guard ⅱ.
    WriteCrashCopy,
    /// Re-reads the answered copy (≤4 MiB) and deletes it only when the hash is the same (N1); if it could not delete or cannot tell whether it is an answered copy, passes the hash on ([`BootPlan::carry_resolved`]).
    RemoveAnsweredCrashCopy { hash: String },
}

/// This run saves no state at all (N3): skips the run marker write (⑤), does not start the recorder, and shutdown only releases the lock without `Final` (N7). The next boot decides again (§12 R16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    StateUnreadable(String),
    /// Same field as ⅰ — `state.json` was written by a version newer than this shell. Neither copied aside, overwritten nor deleted.
    // ADR-0291 R2: copying aside and overwriting would make the newer version lose its own screen state when it comes up again (nobody reads `.corrupt`).
    StateNewer { found: u64 },
    /// ⅱ The copy that had to be made could not be — write failure · copy read IO failure · a newer version's copy that must not be overwritten (N6). One reason line.
    CrashCopyNotWritten(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateAside {
    CopiedAside,
    NotCopied,
}

#[derive(Clone, PartialEq)]
pub struct BootPlan {
    pub model: BootModel,
    /// Boot stage ④ runs these in this order.
    pub actions: Vec<BootAction>,
    pub crash_copy: Option<AwaitingCopy>,
    pub carry_resolved: Option<String>,
    pub guard: Option<Guard>,
    pub state_aside: Option<StateAside>,
    /// `Adopted` if the `state.json` the decision (③) read was usable, else `NotYet` — the recorder's start value is [`saver_claim`] adding ⑤'s result to this.
    pub claim: file::Claim,
}

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

/// Copy column — apart from an unanswered copy it counts as 「none」 and goes by the `state.json` row (§6-5 table).
enum CopyColumn {
    Unanswered(AwaitingCopy),
    AsAbsent(AbsentCopy),
}

enum AbsentCopy {
    Missing,
    /// Read IO failure (I3).
    Unread,
    /// A copy written by a newer version — neither deleted, copied aside nor overwritten (N6).
    Newer,
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
    // I3 decides per file: only the file whose read had an IO failure is neither copied aside, overwritten nor deleted; the other file's action looks only at that file's read. Bundled, a boot that could not read the copy would overwrite an unusable `state.json` with ⑤ without copying it aside.

    // "Answered" exists only in a readable `state.json` — if it cannot be read, the remaining copy counts as unanswered.
    let resolved = match &state {
        StateRead::Usable { file, .. } => file.resolved_crash_copy.clone(),
        _ => None,
    };

    match &state {
        StateRead::IoFailed(reason) => plan.guard = Some(Guard::StateUnreadable(reason.clone())),
        // A newer-version file is in the same field as a read failure — neither copied aside nor overwritten, and this run does not save.
        // ADR-0291 R2
        StateRead::Unusable(Unusable::NewerVersion { found }) => {
            plan.guard = Some(Guard::StateNewer { found: *found })
        }
        StateRead::Unusable(Unusable::Corrupt(_)) => plan.actions.push(BootAction::CopyAsideState),
        StateRead::Missing | StateRead::Usable { .. } => {}
    }

    let copy = match CopyColumn::of(crash_copy, resolved.as_deref()) {
        CopyColumn::Unanswered(awaiting) => {
            // D2-6 · D4: whatever `state.json` is, ask with the default screen and do not overwrite the copy.
            plan.crash_copy = Some(awaiting);
            return plan;
        }
        CopyColumn::AsAbsent(copy) => copy,
    };

    match state {
        StateRead::Usable { file, .. } if file.clean_exit => plan.model = BootModel::Restore(file),
        StateRead::Usable { file, text, .. } => {
            plan.crash_copy = Some(AwaitingCopy {
                hash: codec::crash_copy_hash(&text),
                text,
                file,
            });
            match copy {
                AbsentCopy::Newer => {
                    plan.guard = Some(Guard::CrashCopyNotWritten(
                        "a copy written by a newer version is in place, so it is not overwritten".to_string(),
                    ))
                }
                AbsentCopy::Unread => {
                    plan.guard = Some(Guard::CrashCopyNotWritten(
                        "the copy could not be read, so it is unknown whether overwriting it is fine".to_string(),
                    ))
                }
                AbsentCopy::Unusable => plan
                    .actions
                    .extend([BootAction::CopyAsideCrashCopy, BootAction::WriteCrashCopy]),
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
        // Cannot see whether the answered copy remains — carry the hash on; dropped, the next boot would read a copy that may still remain as unanswered and ask again.
        AbsentCopy::Unread => plan.carry_resolved = resolved,
        AbsentCopy::Missing | AbsentCopy::Newer => {}
    }
    plan
}

/// Boot stage ②–④ — sweeps leftover temporary files, reads and decides `state.json` and the crash copy, and performs the decided actions; failures are logged and it continues. Call it after the shell run lock ([`super::lock::acquire`]) and before the recorder: only after the lock does it read the previous instance's last write, and only before the recorder may it delete temporary files of its own pid.
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
        "boot decision"
    );
    run_actions(files, &mut plan);
    if let Some(guard) = &plan.guard {
        tracing::warn!(
            module = "state",
            guard = ?guard,
            "this run does not save the screen state — the next boot decides again"
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
                        "removed leftover temp files"
                    ),
                    Err(e) => tracing::warn!(
                        module = "state",
                        path = %path.display(),
                        error = %e,
                        "could not remove leftover temp files"
                    ),
                }
            }
        }
        Err(e) => tracing::warn!(
            module = "state",
            error = %e,
            "could not look for leftover temp files — proceeding without cleaning up"
        ),
    }
}

fn report_state_read(state: &StateRead) {
    match state {
        StateRead::Missing => {}
        StateRead::IoFailed(reason) => tracing::warn!(
            module = "state",
            reason = %reason,
            "could not read state.json — treating it as absent"
        ),
        StateRead::Unusable(unusable @ Unusable::NewerVersion { .. }) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.json was written by a newer version than this shell — starting with the default screen, neither copying it aside nor overwriting it (this run does not save the screen state)"
        ),
        StateRead::Unusable(unusable) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.json is unusable — starting with the default screen (the run marker write replaces it)"
        ),
        StateRead::Usable { file, warnings, .. } => {
            warnings
                .iter()
                .for_each(|warning| report_decode_warning(STATE_FILE, warning));
            if !file.clean_exit {
                tracing::warn!(
                    module = "state",
                    "the previous run did not record a normal exit — starting with the default screen"
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
            "could not read state.crash.json — treating it as absent; this boot does not touch the copy"
        ),
        StateRead::Unusable(unusable @ Unusable::NewerVersion { .. }) => tracing::warn!(
            module = "state",
            reason = %unusable,
            "state.crash.json is a copy written by a newer version that this version cannot apply — left as is without asking"
        ),
        StateRead::Unusable(unusable) => tracing::error!(
            module = "state",
            reason = %unusable,
            "state.crash.json is unusable — not asking with that copy"
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
            "skipping one window of the state file"
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
            "skipping one tab of the state file"
        ),
        DecodeWarning::ThemeDropped { window_id, raw } => tracing::warn!(
            module = "state",
            file,
            window_id = %window_id,
            raw = %raw,
            "the window theme in the state file is unknown — following the global theme"
        ),
        DecodeWarning::NumberDropped { window_id, raw } => tracing::warn!(
            module = "state",
            file,
            window_id = %window_id,
            raw = %raw,
            "the window number in the state file is unusable — discarding it"
        ),
    }
}

/// Boot stage ④ — folds into `plan` what ⑤ · ⑥ · the recorder need from the action results (guard ⅱ · the hash to carry on · copying `state.json` aside).
fn run_actions(files: &impl BootFiles, plan: &mut BootPlan) {
    let mut crash_copy_kept_aside = true;
    let replaces_crash_copy = plan.actions.contains(&BootAction::WriteCrashCopy);
    for action in plan.actions.clone() {
        match action {
            BootAction::CopyAsideState => {
                let aside = match files.copy_aside_state() {
                    Ok(to) => {
                        tracing::info!(
                            module = "state",
                            to = %to.display(),
                            "copied the unusable state.json aside"
                        );
                        StateAside::CopiedAside
                    }
                    Err(e) => {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "could not copy the unusable state.json aside — proceeding: the run marker write overwrites the only original (D8)"
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
                    "copied the unusable crash copy aside"
                ),
                Err(e) => {
                    crash_copy_kept_aside = false;
                    if replaces_crash_copy {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "could not copy the unusable crash copy aside — proceeding: the new copy write will soon replace that original without a backup (D8 · ADR-0274 cost 2)"
                        );
                    } else {
                        tracing::error!(
                            module = "state",
                            error = %e,
                            "could not copy the unusable crash copy aside — leaving it without removing: the next boot copies it aside again (D8)"
                        );
                    }
                }
            },
            BootAction::RemoveUnusableCrashCopy if !crash_copy_kept_aside => {}
            BootAction::RemoveUnusableCrashCopy => match files.remove_crash_copy() {
                Ok(()) => tracing::info!(module = "state", "removed the unusable crash copy"),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    tracing::debug!(module = "state", "the unusable crash copy is already gone")
                }
                Err(e) => tracing::warn!(
                    module = "state",
                    error = %e,
                    "could not remove the unusable crash copy — the next boot removes it again"
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
        tracing::error!(
            module = "state",
            "the plan has no raw text to capture the crash copy from — treating it as not captured"
        );
        plan.guard = Some(Guard::CrashCopyNotWritten(
            "the plan has no raw text to capture".to_string(),
        ));
        return;
    };
    match files.write_crash_copy(&copy.text) {
        Ok(()) => tracing::info!(
            module = "state",
            hash = %copy.hash,
            "captured the previous run's screen as the crash copy"
        ),
        Err(e) => {
            tracing::warn!(
                module = "state",
                error = %e,
                "could not capture the crash copy — leaving state.json on disk as is"
            );
            plan.guard = Some(Guard::CrashCopyNotWritten(e.to_string()));
        }
    }
}

/// N1 — re-reads the answered copy and deletes it only when the hash is the same. Returns the hash to carry on (could not delete, or could not re-read so cannot tell whether it is an answered copy). Same branch as the recorder's cleanup (`saver`). The gap between reading and deleting is not atomic — the shell run lock (I1) covers it.
fn remove_answered_copy(files: &impl BootFiles, hash: String) -> Option<String> {
    match files.read_crash_copy() {
        Ok(text) if codec::crash_copy_hash(&text) == hash => match files.remove_crash_copy() {
            Ok(()) => {
                tracing::info!(module = "state", hash = %hash, "removed the answered crash copy");
                None
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tracing::info!(module = "state", hash = %hash, "the answered crash copy is already gone");
                None
            }
            Err(e) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    error = %e,
                    "could not remove the answered crash copy — carrying the hash forward; the writer removes it again"
                );
                Some(hash)
            }
        },
        Ok(_) => {
            tracing::warn!(
                module = "state",
                hash = %hash,
                "the crash copy changed after the decision — it is a new copy, so it is not removed"
            );
            None
        }
        Err(e) => match file::classify(Err(e), &codec::STATE_SPEC) {
            file::Loaded::Missing => {
                tracing::info!(module = "state", hash = %hash, "the answered crash copy is already gone");
                None
            }
            file::Loaded::Parsed(parsed) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    parsed = ?parsed,
                    "the crash copy changed after the decision (unusable content) — not removed"
                );
                None
            }
            file::Loaded::Failed(e) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    error = %e,
                    "could not re-read the crash copy, so it is unknown whether it is the answered copy — carrying the hash forward; the writer looks again"
                );
                Some(hash)
            }
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerOutcome {
    Written,
    Guarded,
    Failed,
}

/// Boot stage ⑤ — writes the run marker (`clean_exit:false`), the windows of the decided model and the hash to carry on directly into `state.json`; the recorder does not exist yet.
/// Call it after ④ and before any window: the original may be overwritten only after copying aside (④) is done, and before windows so that a quietly restored screen killing the app while making a window is read next boot as an abnormal exit (§6-5 ⑤). Failures only log and it proceeds — that run's crash may read as a normal exit (D8 · §12 R14).
/// It decides the current `state.json` again just before writing: if rejected (newer version · unreadable) it does not write, folds the same guard as ③'s guard ⅰ into `plan` and gives `Guarded` (that run does not save, and ⑥'s `restore.status` carries the guard too).
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
            tracing::error!(module = "state", error = %e, "could not write the run marker");
            return MarkerOutcome::Failed;
        }
    };
    if let Err(guard) = judge_before_marker(files, plan) {
        match &guard {
            Guard::StateNewer { found } => tracing::error!(
                module = "state",
                found,
                "the state.json right before writing the run marker was written by a newer version than this shell — not overwriting it; this run does not save the screen state"
            ),
            _ => tracing::warn!(
                module = "state",
                guard = ?guard,
                "could not re-read state.json right before writing the run marker — not overwriting it; this run does not save the screen state"
            ),
        }
        plan.guard = Some(guard);
        return MarkerOutcome::Guarded;
    }
    match files.write_state(&text) {
        Ok(()) => MarkerOutcome::Written,
        Err(e) => {
            tracing::warn!(module = "state", error = %e, "could not write the run marker");
            MarkerOutcome::Failed
        }
    }
}

/// Re-decision just before ⑤'s save — `Ok` if overwriting is allowed (after copying aside if needed; `Ok` even if that fails — D8), else the guard of the same field as ③'s guard ⅰ.
// ADR-0291 R16: do not write holding ③'s decision — a `state.json` that changed to a newer version or became unreadable since must not be overwritten.
fn judge_before_marker(files: &impl BootFiles, plan: &BootPlan) -> Result<(), Guard> {
    let current = file::classify(files.read_state(), &codec::STATE_SPEC);
    match current.write_policy(plan.claim) {
        file::WritePolicy::Write => Ok(()),
        // ④ has just copied this file aside (or failed doing so — D8) within the same boot order, so it is not copied again; a file changed in between is an outside edit while running and unsupported (ADR-0291 C1).
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
                _ => "could not read state.json".to_string(),
            }))
        }
    }
}

fn copy_state_aside_before_marker(files: &impl BootFiles, current: &file::Loaded) {
    let what = match current {
        file::Loaded::Parsed(file::Parsed::Usable { .. }) => "the state.json placed after the decision",
        _ => "the state.json that became unusable after the decision",
    };
    match files.copy_aside_state() {
        Ok(to) => tracing::info!(
            module = "state",
            to = %to.display(),
            "copied {what} aside before overwriting it with the run marker"
        ),
        Err(e) => tracing::error!(
            module = "state",
            "could not copy {what} aside — proceeding: the run marker write overwrites the only original (D8): {e}"
        ),
    }
}

/// The [`file::Claim`] the recorder starts holding — `Adopted` if the `state.json` ③ read was usable or the run marker (⑤) was written; otherwise (missing · corrupt and ⑤ could not write either) `NotYet`, so a usable file placed in between is copied aside before the recorder's first write overwrites it.
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
        let io = || StateRead::IoFailed("locked".into());
        let missing = || StateRead::Missing;
        let answered = |clean| read_of(answered_state_file(clean, &crash_hash()));
        vec![
            // ── present · unanswered → default · not overwritten · asks (D2-6 · D6) ──
            Row::new("unanswered copy × no state", missing(), crash()).asks(crash_file()),
            Row::new("unanswered copy × clean_exit:true", usable(true), crash()).asks(crash_file()),
            Row::new("unanswered copy × clean_exit:false", usable(false), crash()).asks(crash_file()),
            Row::new(
                "unanswered copy × a state that answered another copy",
                read_of(answered_state_file(true, "old copy")),
                crash(),
            )
            .asks(crash_file()),
            Row::new("unanswered copy × unusable state (N6)", broken(), crash())
                .does(vec![CopyAsideState])
                .asks(crash_file()),
            Row::new("unanswered copy × state over the version (guard — not copied aside)", newer(), crash())
                .asks(crash_file())
                .guarded(StateNewer),
            Row::new("unanswered copy × state IO failure (guard ⅰ)", io(), crash())
                .asks(crash_file())
                .guarded(StateUnreadable),
            // ── present · version beyond this one → counted as none · not touched (N6) ──
            Row::new("copy over the version × no state", missing(), newer()),
            Row::new("copy over the version × clean_exit:true", usable(true), newer())
                .restores(state_file(true)),
            Row::new(
                "copy over the version × clean_exit:false (guard ⅱ)",
                usable(false),
                newer(),
            )
            .asks(state_file(false))
            .guarded(CrashCopyNotWritten),
            Row::new("copy over the version × unusable state", broken(), newer()).does(vec![CopyAsideState]),
            Row::new("copy over the version × state IO failure (guard ⅰ)", io(), newer())
                .guarded(StateUnreadable),
            Row::new("copy over the version × state over the version (guard)", newer(), newer()).guarded(StateNewer),
            // ── none ──
            Row::new("no copy × no state", missing(), missing()),
            Row::new("no copy × clean_exit:true", usable(true), missing())
                .restores(state_file(true)),
            Row::new(
                "no copy × clean_exit:true carrying an answered hash (no hash to carry forward)",
                read_of(answered_state_file(true, "h")),
                missing(),
            )
            .restores(answered_state_file(true, "h")),
            Row::new("no copy × clean_exit:false (D5)", usable(false), missing())
                .does(vec![WriteCrashCopy])
                .asks(state_file(false)),
            Row::new("no copy × unusable state", broken(), missing()).does(vec![CopyAsideState]),
            Row::new(
                "no copy × state over the version (guard — neither copied aside nor overwritten · ADR-0291 R2)",
                newer(),
                missing(),
            )
            .guarded(StateNewer),
            Row::new("no copy × state IO failure (guard ⅰ)", io(), missing()).guarded(StateUnreadable),
            // ── present · answered → counted as none · deleted only when the hash is the same (I2 · N1) ──
            Row::new("answered copy × clean_exit:true", answered(true), crash())
                .restores(answered_state_file(true, &crash_hash()))
                .does(vec![RemoveAnsweredCrashCopy { hash: crash_hash() }]),
            Row::new(
                "answered copy × clean_exit:false (the new copy replaces it)",
                answered(false),
                crash(),
            )
            .does(vec![WriteCrashCopy])
            .asks(answered_state_file(false, &crash_hash())),
            // ── unusable copy → deleted after copying aside (ADR-0274) ──
            Row::new("unusable copy × no state", missing(), broken())
                .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy]),
            Row::new("unusable copy × clean_exit:true", usable(true), broken())
                .restores(state_file(true))
                .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy]),
            Row::new(
                "unusable copy × clean_exit:false (new copy after copying aside)",
                usable(false),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, WriteCrashCopy])
            .asks(state_file(false)),
            Row::new(
                "unusable copy (over the cap) × unusable state",
                broken(),
                StateRead::Unusable(Unusable::Corrupt("over the cap".into())),
            )
            .does(vec![
                CopyAsideState,
                CopyAsideCrashCopy,
                RemoveUnusableCrashCopy,
            ]),
            Row::new(
                "unusable copy × state IO failure (guard ⅰ — the copy side looks only at the copy read)",
                io(),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy])
            .guarded(StateUnreadable),
            Row::new(
                "unusable copy × state over the version (guard — the copy side looks only at the copy read)",
                newer(),
                broken(),
            )
            .does(vec![CopyAsideCrashCopy, RemoveUnusableCrashCopy])
            .guarded(StateNewer),
            // ── copy read IO failure → counted as none · zero actions on the copy side (I3 — per file) ──
            Row::new("copy IO failure × no state", missing(), io()),
            Row::new("copy IO failure × clean_exit:true", usable(true), io())
                .restores(state_file(true)),
            Row::new(
                "copy IO failure × clean_exit:true carrying an answered hash (carries the hash forward)",
                read_of(answered_state_file(true, "h")),
                io(),
            )
            .restores(answered_state_file(true, "h"))
            .carries("h"),
            Row::new(
                "copy IO failure × clean_exit:false (guard ⅱ)",
                usable(false),
                io(),
            )
            .asks(state_file(false))
            .guarded(CrashCopyNotWritten),
            Row::new(
                "copy IO failure × clean_exit:false carrying an answered hash (guard ⅱ — ⑤ is skipped, so the hash stays on disk)",
                read_of(answered_state_file(false, "h")),
                io(),
            )
            .asks(answered_state_file(false, "h"))
            .guarded(CrashCopyNotWritten),
            Row::new(
                "copy IO failure × unusable state (the state side looks only at the state read — copied aside)",
                broken(),
                io(),
            )
            .does(vec![CopyAsideState]),
            Row::new(
                "copy IO failure × state over the version (guard — not copied aside)",
                newer(),
                io(),
            )
            .guarded(StateNewer),
            Row::new("copy IO failure × state IO failure (guard ⅰ)", io(), io()).guarded(StateUnreadable),
        ]
    }

    #[test]
    fn the_boot_table_decides_every_row() {
        let rows = rows();
        assert_eq!(rows.len(), 36, "row count — when adding or removing rows, fix this number too");
        for row in rows {
            let claim = match row.state {
                StateRead::Usable { .. } => file::Claim::Adopted,
                _ => file::Claim::NotYet,
            };
            let plan = decide_boot(BootInputs {
                state: row.state,
                crash_copy: row.crash_copy,
            });
            assert_eq!(plan.claim, claim, "{} — the writer's start decision axis", row.name);
            assert_eq!(plan.model, row.model, "{} — model", row.name);
            assert_eq!(plan.actions, row.actions, "{} — actions", row.name);
            assert_eq!(plan.crash_copy, row.ask, "{} — asks", row.name);
            assert_eq!(
                plan.carry_resolved, row.carry,
                "{} — hash to carry forward",
                row.name
            );
            assert_eq!(guard_kind(&plan.guard), row.guard, "{} — guard", row.name);
            assert_eq!(
                plan.state_aside, None,
                "{} — ④ sets the copy-aside result",
                row.name
            );
        }
    }

    #[test]
    fn a_corrupt_state_file_of_any_kind_is_copied_aside() {
        for reason in ["over the cap", "not JSON", "not a state file shape"] {
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
            decide(StateRead::IoFailed("locked".into())).guard,
            Some(Guard::StateUnreadable("locked".into()))
        );
    }

    // ── sorting read results ──

    #[test]
    fn read_errors_split_into_missing_unusable_and_io_failure() {
        let from_err = |e: io::Error| StateRead::from_read(Err(e));
        assert_eq!(from_err(io::ErrorKind::NotFound.into()), StateRead::Missing);
        assert!(matches!(
            from_err(io::Error::new(io::ErrorKind::InvalidData, "over the cap")),
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
            let failure = fails.then(|| io::Error::other(format!("fake failure: {call}")));
            self.calls.borrow_mut().push(call);
            failure.map_or(Ok(()), Err)
        }
    }

    impl BootFiles for FakeFiles {
        // Remains even when read like a disk — ③'s read and ⑤'s re-decision look at the same field and the test changes it in between. Empty field = no file.
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
                Err(io::Error::other("fake folder read failure"))
            } else {
                Ok(vec![
                    (PathBuf::from("state.json.tmp1.0"), Ok(())),
                    (
                        PathBuf::from("state.json.tmp2.0"),
                        Err(io::Error::other("fake remove failure")),
                    ),
                ])
            }
        }
    }

    fn marker_of(files: &FakeFiles) -> StateFile {
        let written = files.written();
        assert_eq!(written.len(), 1, "the run marker is written once");
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

        assert_eq!(plan.guard, None, "a copy-aside failure is not a guard (D8)");
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
            "copying the unusable copy aside is not state.json's copy-aside"
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
            "state.json copied aside · written 0 times — an unusable copy looks only at the copy read, is copied aside, then removed (I3 is per file)"
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
            "copy = the raw bytes read, as is"
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
            "the copy write comes before the run marker"
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
            "restore source = the state.json raw text in memory (L2)"
        );
        assert_eq!(
            write_run_marker(&files, &mut plan, vec![a_window()], 9),
            MarkerOutcome::Guarded
        );
        assert!(files.written().is_empty(), "state.json on disk as is");
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
            "copy-aside before the run marker · ⑤'s re-decision does not copy aside again · 0 actions on the copy side (I3 is per file)"
        );
    }

    /// ADR-0291 R2: a newer-version `state.json` is neither copied aside nor overwritten and this run does not save; the copy side looks only at the copy read (I3 is per file).
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
            "state.json copied aside · written 0 times"
        );
    }

    /// ADR-0291 R16: ⑤ does not write holding ③'s decision — if the file became a newer version · unreadable in between, it is neither copied aside nor overwritten and ③'s guard ⅰ field is carried into the plan (this run does not save).
    #[test]
    fn the_marker_never_overwrites_a_state_file_that_turned_newer_or_unreadable() {
        for (now, guard) in [
            (Ok(newer_text()), Guard::StateNewer { found: 2 }),
            (
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked")),
                Guard::StateUnreadable("locked".into()),
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
                "copied aside · written 0 times — {label}"
            );
        }
    }

    /// ADR-0291 R8 · R16: a file not present in ③ but placed before ⑤ (corrupt or usable — D14) is written after copying aside; if copying aside fails it still writes (D8).
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
            "the file did not exist and ⑤ could not write either — a file placed in between is not this run's"
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
        assert!(files.crash_written().is_empty(), "the copy is not overwritten (D2-6)");
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
                "neither removed, copied aside nor overwritten (N6) — clean_exit:{}",
                state.clean_exit
            );
            assert_eq!(
                plan.guard.is_some(),
                !state.clean_exit,
                "failing to capture a copy that should be captured is guard ⅱ"
            );
        }
    }

    // ── unusable copy ──

    #[test]
    fn an_unusable_copy_is_copied_aside_then_removed() {
        let files =
            FakeFiles::reading_both(Err(io::ErrorKind::NotFound.into()), vec![Ok("{".into())]);

        let plan = prepare(&files);

        assert_eq!(plan.crash_copy, None, "not asking with that copy");
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
            "a copy-aside failure of the copy is not state.json's"
        );
        assert_eq!(
            files.calls(),
            ["sweep", "read", "read_crash", "copy_aside_crash"],
            "an original with nowhere to go is not removed"
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
                "state failure {state_fails} · copy failure {crash_fails}"
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

            assert_eq!(plan.state_aside, Some(expected), "failure {fails}");
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
        assert_eq!(plan.crash_copy, None, "not asking with that copy");
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
            "another copy — the writer must not remove it either"
        );
    }

    #[test]
    fn an_answered_copy_that_became_unusable_before_the_delete_is_not_removed() {
        let files = answered_boot(vec![
            Ok(crash_text()),
            Err(io::Error::new(io::ErrorKind::InvalidData, "over the cap")),
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
        assert_eq!(plan.guard, None, "failing to remove is not a guard");
        assert_eq!(marker, MarkerOutcome::Written);
        assert_eq!(
            marker_of(&files).resolved_crash_copy,
            Some(crash_hash()),
            "the answer stays on disk (I2)"
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
            "not removed; the new copy write replaces it"
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
                .expect("clock")
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
            "the run marker replaced it"
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
            "guard ⅱ — state.json as is"
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
