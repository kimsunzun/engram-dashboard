//! Mount rename (`library.renameMount {mount, to, yes?, root_generation?, plan_digest?}`) — an inherent [`LibraryService`]
//! method:
//! the mount `from` takes the name `to`, and every reference to `from/…` in the preset files of every readable mount
//! and in the agents' attachments is re-pointed at `to/…` (ADR-0400 decision 12).
//!
//! ## Rules (invariants)
//!
//! - **Judged before anything is written, under the mount-change mutex.** The mutex is taken through the load-time
//!   judgement ([`LibraryService::take_mount_change`]); then a read-only `library.json` refuses, the first scan is
//!   waited for (the plan needs every mount's items), the names are judged, the mount itself must stat `Ok` (a mount
//!   held as overlapping answers its hold), every other mount that cannot be read is walked again, every other mount
//!   recorded readable that holds a file to rewrite is stat'ed fresh (one not `Ok` is recorded so, without a ticket —
//!   a folder lost since its last walk), and the plan is built ([`RenamePlan`]). A plan that touches an agent while
//!   the profile store is `ReadOnly` refuses with the store's
//!   refusal (ADR-0291 · ADR-0379). A plan that rewrites files, or leaves a reference to `from` unrewritten — a
//!   referencing file in a mount that cannot be read (under `unread`, not in `files`), or a mount never walked this run
//!   (what in it refers to `from` is unknown) — needs `yes`, the plan's `root_generation` and its `plan_digest`
//!   ([`confirmation_digest`] over both sides), so a confirmation covers only what the caller saw: a call whose own
//!   plan differs answers `mount-rename-unconfirmed` with that plan. A plan with neither asks nothing — but a
//!   `plan_digest` the caller sent is still compared, so a plan that became empty since is answered again. The plan
//!   and the hold sort the files into the sides with one classifier ([`Sides`]).
//! - **The plan is judged again inside the hold**: the files the hold would rewrite and those it would leave unread
//!   must be the plan's — the digest equal to the plan's — or nothing is applied and the caller answers
//!   `mount-rename-unconfirmed` with a fresh plan (an edit, a scan or a lost mount landed between the plan and the
//!   hold).
//! - **One primitive, one hold** ([`Gate::rename_mount`] — a `Root` ticket and one write lock): the preset files are
//!   rewritten one at a time through the edit pipeline ([`LibraryService::rewrite_in`] — whole files, ADR-0358), then
//!   the root generation and the change number are raised, then the agents are re-pointed once through
//!   `ProfileEditPort::rewrite_preset_paths` (pairs built from the registry read after the raise — an `agent.new`
//!   inserted before the raise is in it; one after it meets the raised number and judges again, ADR-0400 decision
//!   18), then the catalog range is re-keyed and the registration renamed. The save follows the hold.
//! - **A per-file refusal skips the file; an IO failure stops** (relink's rule, ADR-0335): a read or write failure stops
//!   the walk before the agents and the registration — the files written point at `to`, `from` stays registered (a
//!   stop before any write answers that failure's refusal). A store refusal at the agents step comes after the raise:
//!   the answer carries `stopped` and the new root generation, `from` stays registered. A file not found whose mount
//!   folder no longer stats `Ok` (lost during the hold) stops the walk too, and that mount is recorded so after the
//!   hold; one whose folder is there is skipped. A file in a mount that cannot be read is skipped with
//!   `library-unavailable`, and a mount never walked this run is one skipped row at its top (ADR-0400 decision 6).
//! - **The repeat after a stop is the ordinary call** (ADR-0407): a stopped answer carries no plan digest, and nothing
//!   in it confirms a later call. The repeat plans from the files as they are now — those already rewritten no longer
//!   refer to `from` and drop out, a mount recorded lost is named under `unread` — and a plan of what is left that
//!   needs confirmation is answered to confirm: the tokens confirmed before the stop answer `mount-rename-unconfirmed`
//!   (or `root-generation-stale` once the agents step raised the generation) with it — unless that plan is the
//!   confirmed one again (every file written came to refer to `from` again: the digest pins paths, not content). No
//!   progress is stored (ADR-0335).
//! - **Holds.** A mount held as overlapping another at load stats unreadable, so it is refused `mount-unusable` before
//!   anything: a rename never moves a hold, and no hold names `to` (holds are set only on registered mounts at load,
//!   and a removal ends its mount's).
//! - **Repeat after a failed save** (ADR-0400 decision 14): `from` not registered, `to` live, `library.json` `unsaved`
//!   and its last saved list naming `from` at the live `to`'s folder — judged before any generation comparison. No
//!   readable file holding `from/…` → only the save (files left unread ask nothing here: the rename is already live,
//!   and refusing the save would not re-point them); files still do → the plan with the current root generation, and the
//!   confirmed call rewrites the references (no re-key, no registration) and saves. A rename to the name the mount
//!   already has (also a case-only rename repeated where path keys ignore case) saves again.
// ADR-0335
// ADR-0358
// ADR-0379
// ADR-0400 decisions 12, 14, 16 and 18
// ADR-0407

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use engram_dashboard_agent::profile::{StoreError, StoreStatus};
use engram_dashboard_agent::profile_edit::{Attachments, PresetPathRename};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::{path_key, same_path, PathKey};
use serde_json::{json, Value};

use crate::agent_edit::port_refused;
use crate::catalog::{preset_refs, Catalog, ItemContent};
use crate::edit::{as_read, list_mut, ITEM_LISTS};
use crate::folder::{FolderStatus, LaunchIo, Need};
use crate::format::{PresetDoc, PresetLists};
use crate::gate::RenameEnd;
use crate::launch::sha256_hex;
use crate::mounts::{moved_to, Mount, MountName};
use crate::path::{AssetRef, LibPath};
use crate::relink::{RelinkSkip, RelinkStop};
use crate::resolve::ref_key;
use crate::{
    mount_name_invalid, mount_name_taken, mount_not_found, mount_unavailable, same_folder,
    LibraryRefusal, LibraryService, LibraryStatus, MountSeen, MountState, MountsNow, Pointing,
    RefusalClass, MOUNT_RENAME_UNCONFIRMED, MOUNT_UNUSABLE, NOT_FOUND, ROOT_GENERATION_STALE,
};

/// The answer of [`LibraryService::rename_mount`].
#[derive(Debug)]
pub struct RenameAnswer {
    /// The status after the rename (or after the stop).
    pub status: LibraryStatus,
    /// The references re-pointed, `(old, new)` as spelled from the virtual root, each once: in a preset file written,
    /// or by the agents' rewrite.
    pub fixed: Vec<(String, String)>,
    /// The preset files written, in walk (path) order.
    pub written: Vec<String>,
    /// The preset files skipped, each with its refusal; a row whose path is a mount's name: that mount could not be
    /// read at all.
    pub skipped: Vec<RelinkSkip>,
    /// Why it stopped part-way — `from` is still registered; `None` = renamed.
    pub stopped: Option<RelinkStop>,
    /// What already pointed at `to` before the rename (it now resolves into the renamed mount); nothing when the two
    /// names are the same by path key.
    pub already_pointing: Pointing,
}

/// What a rename would do, built before anything is written (ADR-0400 decision 12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenamePlan {
    pub from: String,
    pub to: String,
    /// The preset files to rewrite, per mount, in list order.
    pub files: Vec<PlanMount>,
    /// The mounts that cannot be read — their references to `from` stay as they are (fixed by hand, ADR-0396
    /// decision 3).
    pub unread: Vec<PlanUnread>,
    /// The agents with an attachment in `from`, `(id, name)`.
    pub agents: Vec<(AgentId, String)>,
    /// What already points at `to` and will resolve into the renamed mount ([`RenameAnswer::already_pointing`]).
    pub already_pointing: Pointing,
    /// The root generation the plan was built at — a confirmed call carries it.
    pub root_generation: u64,
    /// The plan's [`Sides::digest`] — a confirmed call carries it.
    pub plan_digest: String,
}

/// A mount a rename cannot read, as its plan names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanUnread {
    pub mount: String,
    pub state: MountState,
    /// Its preset files that refer to `from` — left as they are; `None` = never walked this run and none known, so
    /// what in it refers to `from` is unknown.
    pub paths: Option<Vec<String>>,
}

/// The files of one mount a rename would rewrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanMount {
    pub mount: String,
    /// The mount's folder is in a git working tree (a `.git` folder or file in it or a parent); `None` = could not
    /// be told (a stat gave no answer).
    pub git: Option<bool>,
    pub paths: Vec<String>,
}

impl RenamePlan {
    /// The plan as a refusal's `plan` argument; `whole_files` says the files are rewritten whole (ADR-0358).
    pub fn to_json(&self) -> Value {
        json!({
            "mount": self.from,
            "to": self.to,
            "files": self.files.iter().map(|mount| json!({
                "mount": mount.mount,
                "git": mount.git,
                "paths": mount.paths,
            })).collect::<Vec<_>>(),
            "unread": self.unread.iter().map(|unread| json!({
                "mount": unread.mount,
                "state": unread.state.word(),
                "why": unread.state.why(),
                "paths": unread.paths,
            })).collect::<Vec<_>>(),
            "agents": self.agents.iter().map(|(id, name)| json!({
                "id": id.to_string(),
                "name": name,
            })).collect::<Vec<_>>(),
            "whole_files": true,
            "already_pointing": {
                "presets": self.already_pointing.presets,
                "agents": self.already_pointing.agents,
            },
        })
    }

    fn rewrites_files(&self) -> bool {
        self.files.iter().any(|mount| !mount.paths.is_empty())
    }

    /// Whether a reference to `from` would stay as it is: a referencing file in a mount that cannot be read, or a
    /// mount never walked (its references unknown).
    fn leaves_unread(&self) -> bool {
        self.unread
            .iter()
            .any(|unread| unread.paths.as_ref().is_none_or(|paths| !paths.is_empty()))
    }

    /// Whether the rename needs `yes` with the plan's tokens: it rewrites files or leaves references unread.
    fn needs_confirmation(&self) -> bool {
        self.rewrites_files() || self.leaves_unread()
    }

    /// The `plan_digest` a confirmed call carries ([`Sides::digest`] of the state the plan was built from).
    pub fn digest(&self) -> String {
        self.plan_digest.clone()
    }
}

/// Where each preset file referring to `from` stands, judged from the recorded state (the catalog and what the scans
/// know of each mount) — the one classifier both the plan and the hold use, so on the same state their digests agree
/// (two of them could disagree and make every confirmed call refuse).
struct Sides {
    /// The referencing files in mounts that can be read — rewritten.
    rewrite: Vec<LibPath>,
    /// The referencing files in mounts that cannot be read — left as they are.
    unread: Vec<LibPath>,
    /// The mounts that cannot be read, in list order, with what the scans know of each.
    unusable: Vec<(Mount, MountSeen)>,
}

impl Sides {
    /// A referencing file in no registered mount is on neither side — nothing could rewrite it.
    fn of(catalog: &Catalog, mounts: &MountsNow, from: &MountName, to: &MountName) -> Sides {
        let unusable: Vec<(Mount, MountSeen)> = mounts
            .iter()
            .map(|mount| (mount.clone(), mounts.seen(&mount.name)))
            .filter(|(_, seen)| !seen.usable())
            .collect();
        let mut rewrite = Vec::new();
        let mut unread = Vec::new();
        for file in referencing_files(catalog, from, to) {
            let Some((mount, _)) = mounts.locate(&file) else {
                continue;
            };
            match unusable
                .iter()
                .any(|(held, _)| held.name.key() == mount.name.key())
            {
                true => unread.push(file),
                false => rewrite.push(file),
            }
        }
        Sides {
            rewrite,
            unread,
            unusable,
        }
    }

    /// The unusable mount `file` lies in (by name key).
    fn unusable_of(&self, file: &LibPath) -> Option<&(Mount, MountSeen)> {
        self.unusable
            .iter()
            .find(|(mount, _)| in_mount(file, &mount.name))
    }

    /// [`confirmation_digest`] of the three sides: the files rewritten, the files left unread and the mounts never
    /// walked (their references unknown).
    fn digest(&self) -> String {
        confirmation_digest(
            self.rewrite.iter().map(LibPath::as_str),
            self.unread.iter().map(LibPath::as_str),
            self.unusable
                .iter()
                .filter(|(_, seen)| !seen.walked)
                .map(|(mount, _)| mount.name.as_str()),
        )
    }
}

#[cfg(test)]
thread_local! {
    /// Run once on the renaming thread right after the rename's hold is released — a test lands a change there.
    pub(crate) static AFTER_HOLD: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// What the rename's body did inside the hold.
struct Done {
    fixed: Vec<(String, String)>,
    written: Vec<String>,
    skipped: Vec<RelinkSkip>,
    stopped: Option<RelinkStop>,
}

impl LibraryService {
    /// Renames the mount `mount` to `to` and re-points every reference to it (module header). `yes` confirms a plan
    /// that rewrites files or leaves references unread, with the `root_generation` and the `plan_digest` it answered.
    /// Answers the status after the rename and what was fixed, written, skipped, why it stopped, and what already
    /// pointed at `to`.
    ///
    /// Refusals, nothing applied: the built-in `engram` as `mount` `Conflict` · `mount-read-only`; a name the rules
    /// refuse `InvalidArgument` · `mount-name-invalid`; the load-time judgement unfinished `Unavailable` ·
    /// `library-unavailable`; a read-only `library.json` `Unavailable` · `library-config-read-only`; no first scan
    /// applied `Unavailable` · `library-unavailable`; `mount` not registered (outside the repeat) `NotFound` ·
    /// `mount-not-found`; `to` another mount's name `Conflict` · `mount-name-taken`; the mount cannot be read `Conflict`
    /// · [`MOUNT_UNUSABLE`]; files to rewrite or references left unread without `yes`, `root_generation` or
    /// `plan_digest`, a `plan_digest` other than this call's plan's (also when that plan needs no confirmation), or
    /// that plan changed before its hold `Conflict` ·
    /// [`MOUNT_RENAME_UNCONFIRMED`] `{plan,
    /// root_generation, plan_digest}`; a stale `root_generation` `Conflict` · `root-generation-stale` `{given,
    /// current, plan, plan_digest}`; agents to change while the port cell is empty `Internal` ·
    /// `profile-edit-unavailable`; a stop before any file
    /// was written — its own refusal. After the rename, or for a rename to the name the mount already has: a save
    /// refused or failed `Unavailable` · `library-config-write-failed` (the rename stays live; repeating the call saves
    /// it).
    // ADR-0400 decisions 12 and 14
    pub fn rename_mount(
        &self,
        mount: &str,
        to: &str,
        yes: bool,
        root_generation: Option<u64>,
        plan_digest: Option<&str>,
    ) -> Result<RenameAnswer, LibraryRefusal> {
        let from = self.named_mount(mount)?;
        let to = MountName::parse(to).map_err(|why| mount_name_invalid(to, &why))?;
        // ADR-0360: the load-time judgement and the rename's own folder asks share one read budget.
        let read = self.io.begin_read();
        let _serial = self.take_mount_change(&read)?;
        self.refuse_read_only_config()?;
        // The plan names every mount's references: it waits for the whole first scan, as `query` does.
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        let (live_from, live_to) = self.gate.with_read(|state| {
            (
                state.mounts.get(from.as_str()).cloned(),
                state.mounts.get(to.as_str()).cloned(),
            )
        });
        let register = match (&live_from, &live_to) {
            (Some(live), Some(other)) if live.name.key() != other.name.key() => {
                return Err(mount_name_taken(&other.name));
            }
            (Some(live), _) if live.name.as_str() == to.as_str() => {
                // ADR-0400 decision 14: already so named — on a platform whose path keys ignore case, also the repeat
                //   of a case-only rename whose save failed (`from` finds the live `to`): success, and saved again.
                self.save_mounts_or_refuse(&rename_unsaved(&from, &to))?;
                return Ok(RenameAnswer {
                    already_pointing: self.newly_pointing(&from, &to),
                    status: self.status(),
                    fixed: Vec::new(),
                    written: Vec::new(),
                    skipped: Vec::new(),
                    stopped: None,
                });
            }
            (Some(_), _) => true,
            // ADR-0400 decision 14: the repeat of a rename whose save failed — the last saved list still names `from`
            //   at the folder the live `to` has (a rename keeps its folder; a failed removal of `from` or a rename of
            //   it to a third name is not this rename).
            (None, Some(live))
                if self.config.unsaved_rename(&from).is_some_and(|place| {
                    same_folder(&place.resolve(&self.default_place()), &live.folder)
                }) =>
            {
                false
            }
            (None, _) => return Err(mount_not_found(&from)),
        };
        if let Some(live) = &live_from {
            // The mount itself must be readable — its own references are rewritten too.
            let status = self
                .stat_mount_list(&read, std::slice::from_ref(live))
                .remove(0);
            if status != FolderStatus::Ok {
                return Err(mount_unusable(&live.name, &MountState::of(&status)));
            }
        }
        // The mounts recorded readable that hold a file to rewrite, `from` aside (stat'ed above).
        let (names, referencing) = self.gate.with_read(|state| {
            let names: Vec<MountName> = state
                .mounts
                .iter()
                .map(|mount| mount.name.clone())
                .collect();
            let mut referencing: Vec<Mount> = Vec::new();
            for file in referencing_files(&state.catalog, &from, &to) {
                let Some((mount, _)) = state.mounts.locate(&file) else {
                    continue;
                };
                let known = referencing
                    .iter()
                    .any(|held| held.name.key() == mount.name.key());
                if !known && mount.name.key() != from.key() && state.seen(&mount.name).usable() {
                    referencing.push(mount.clone());
                }
            }
            (names, referencing)
        });
        self.rewalk_unusable(&names);
        // ADR-0400 decisions 12 and 17: a mount recorded readable whose folder is gone since its last walk (a drive or
        //   share lost, no scan since) is stat'ed fresh before the plan and recorded so — the plan names it under
        //   `unread` and its files are skipped, instead of every repeat stopping at its first file.
        if !referencing.is_empty() {
            self.gate
                .observe(referencing, &|mounts| self.scan_stats(mounts));
        }
        let plan = self.rename_plan(&read, &from, &to);
        // ADR-0291 · ADR-0379: a read-only agent store refuses before any file is written.
        if !plan.agents.is_empty() {
            if let StoreStatus::ReadOnly(refusal) = self.values.registry.store_status() {
                return Err(LibraryRefusal::Store(StoreError::Refused(refusal)));
            }
        }
        if !register && !plan.rewrites_files() {
            // The repeat with nothing left to rewrite: only the save, no generation compared — references left unread
            //   ask nothing here, as the rename is already live.
            self.save_mounts_or_refuse(&rename_unsaved(&from, &to))?;
            tracing::info!(
                mount = from.as_str(),
                to = to.as_str(),
                "library mount rename saved on a repeat"
            );
            return Ok(RenameAnswer {
                already_pointing: plan.already_pointing,
                status: self.status(),
                fixed: Vec::new(),
                written: Vec::new(),
                skipped: Vec::new(),
                stopped: None,
            });
        }
        // The confirmation pins what the caller saw on both sides, not only the generation: a plan that differs from
        //   the confirmed one is answered again.
        let digest = plan.digest();
        if plan.needs_confirmation() {
            match (yes, root_generation, plan_digest) {
                (true, Some(given), _) if given != plan.root_generation => {
                    return Err(stale_with_plan(given, &plan));
                }
                (true, Some(_), Some(seen)) if seen == digest => {}
                _ => return Err(unconfirmed(&plan)),
            }
        } else if plan_digest.is_some_and(|seen| seen != digest) {
            // A confirmation of a plan that is no longer this one — this one may have nothing left to confirm, but the
            //   caller did not see it: answered again (the next call, with no tokens, runs).
            return Err(unconfirmed(&plan));
        }
        let mut lost = None;
        let outcome = self
            .gate
            .rename_mount(
                &from,
                &to,
                register,
                root_generation,
                |catalog, mounts, seq, raise| {
                    self.rename_in(
                        &from, &to, register, &digest, catalog, mounts, seq, raise, &mut lost,
                    )
                },
            )
            .map_err(|current| {
                stale_with_plan(
                    root_generation.unwrap_or(current),
                    &self.rename_plan(&read, &from, &to),
                )
            })?;
        #[cfg(test)]
        if let Some(hook) = AFTER_HOLD.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
        // ADR-0400 decision 17: the mount the walk found gone is recorded so, without a ticket — the repeat's plan names
        //   it under `unread` and skips its files.
        if let Some((mount, status, at)) = lost {
            self.gate.record_unusable(vec![(mount, status)], at);
        }
        let mut done = match outcome {
            // The files changed between the plan and the hold: asked again, with a fresh plan.
            Err(LibraryRefusal::Refused { reason, .. })
                if reason.code == MOUNT_RENAME_UNCONFIRMED =>
            {
                return Err(unconfirmed(&self.rename_plan(&read, &from, &to)));
            }
            outcome => outcome?,
        };
        if register && done.stopped.is_none() {
            // The files of the renamed mount are answered by the paths they have now.
            let now_named = |path: &mut String| {
                if let Some(new) = LibPath::parse(path)
                    .ok()
                    .and_then(|old| moved_to(&old, &from, &to))
                {
                    *path = new.as_str().to_owned();
                }
            };
            done.written.iter_mut().for_each(&now_named);
            done.skipped
                .iter_mut()
                .for_each(|skip| now_named(&mut skip.path));
        }
        if !done.skipped.is_empty() {
            tracing::warn!(
                mount = from.as_str(),
                to = to.as_str(),
                skipped = done.skipped.len(),
                "library mount rename skipped files"
            );
        }
        match &done.stopped {
            None => {
                tracing::info!(
                    mount = from.as_str(),
                    to = to.as_str(),
                    written = done.written.len(),
                    fixed = done.fixed.len(),
                    "library mount renamed"
                );
                self.save_mounts_or_refuse(&rename_unsaved(&from, &to))?;
            }
            Some(stop) => tracing::warn!(
                mount = from.as_str(),
                to = to.as_str(),
                written = done.written.len(),
                refusal = ?stop.refusal,
                "library mount rename stopped; the old name stays"
            ),
        }
        Ok(RenameAnswer {
            status: self.status(),
            fixed: done.fixed,
            written: done.written,
            skipped: done.skipped,
            stopped: done.stopped,
            already_pointing: plan.already_pointing,
        })
    }

    /// The body of the rename's hold ([`Gate::rename_mount`]): judged again from the recorded state, then the preset
    /// files, the raise, and the agents (module header). Answers what it did, the keys it applied, and whether the
    /// rename goes on. `planned` = this call's plan's [`confirmation_digest`] (confirmed by the caller when the plan
    /// needed it); files to rewrite or left unread other than its (an edit, a scan or a lost mount landed after the
    /// plan) refuse [`MOUNT_RENAME_UNCONFIRMED`] with no plan — the caller builds it, outside the hold. A mount found gone ([`LibraryService::mount_gone`]) is left in `lost` with
    /// its stat and the sequence drawn before it, for the caller to record after the hold.
    #[allow(clippy::too_many_arguments)]
    fn rename_in(
        &self,
        from: &MountName,
        to: &MountName,
        register: bool,
        planned: &str,
        catalog: &mut Catalog,
        mounts: &MountsNow,
        seq: &dyn Fn() -> u64,
        raise: &dyn Fn() -> u64,
        lost: &mut Option<(Mount, FolderStatus, u64)>,
    ) -> (Result<Done, LibraryRefusal>, Vec<PathKey>, RenameEnd) {
        let mut touched = Vec::new();
        let refuse = |refusal| (Err(refusal), Vec::new(), RenameEnd::Stop);
        if register {
            // ADR-0400 decision 6: a scan queued ahead of this ticket may have found the mount gone.
            let seen = mounts.seen(from);
            if !seen.usable() {
                return refuse(mount_unusable(from, &seen.shown()));
            }
        }
        let port = match agents_in(&self.values.registry.attachments(), from) {
            true => match self.profile_edit() {
                Ok(port) => Some(port),
                Err(refusal) => return refuse(refusal),
            },
            false => self.profile_edit().ok(),
        };
        let sides = Sides::of(catalog, mounts, from, to);
        if sides.digest() != planned {
            return refuse(LibraryRefusal::new(
                RefusalClass::Conflict,
                MOUNT_RENAME_UNCONFIRMED,
                json!({}),
            ));
        }
        let reason =
            |(mount, seen): &(Mount, MountSeen)| mount_unavailable(&mount.name, &seen.shown());
        let mut done = Done {
            fixed: Vec::new(),
            written: Vec::new(),
            skipped: Vec::new(),
            stopped: None,
        };
        // ADR-0400 decision 6: a mount never walked this run has no files in the catalog — one skipped row at its top.
        let mut unknown: Vec<RelinkSkip> = sides
            .unusable
            .iter()
            .filter(|(_, seen)| !seen.walked)
            .map(|held| RelinkSkip {
                path: held.0.name.as_str().to_owned(),
                reason: reason(held),
            })
            .collect();
        unknown.sort_by(|a, b| a.path.cmp(&b.path));
        done.skipped.extend(unknown);
        // ADR-0400 decision 12: a file in a mount that cannot be read is skipped with that mount's reason.
        for file in &sides.unread {
            if let Some(held) = sides.unusable_of(file) {
                done.skipped.push(RelinkSkip {
                    path: file.as_str().to_owned(),
                    reason: reason(held),
                });
            }
        }
        // The mounts stat'ed `Ok` after a file in them read as not found.
        let mut present: HashSet<PathKey> = HashSet::new();
        for file in sides.rewrite {
            let rewritten = self.rewrite_in(
                &file,
                None,
                catalog,
                mounts,
                seq,
                |edit| Ok(moved_refs(edit.doc, from, to)),
                &mut touched,
            );
            // ADR-0335: a file not found whose mount folder is gone too (a share or drive lost during the hold) is a
            //   failure of the mount, not of the file — the walk stops, as the up-front check would have refused.
            let not_found = matches!(
                &rewritten,
                Err(LibraryRefusal::Refused { reason, .. }) if reason.code == NOT_FOUND
            );
            let rewritten = match rewritten {
                Err(refusal) if not_found => Err(self
                    .mount_gone(mounts, &file, seq, &mut present, lost)
                    .unwrap_or(refusal)),
                rewritten => rewritten,
            };
            match rewritten {
                Ok(rewritten) => {
                    if rewritten.written {
                        done.written.push(file.as_str().to_owned());
                        done.fixed.extend(rewritten.answer);
                    }
                }
                Err(LibraryRefusal::Refused { class, reason })
                    if class != RefusalClass::Unavailable =>
                {
                    done.skipped.push(RelinkSkip {
                        path: file.as_str().to_owned(),
                        reason,
                    });
                }
                Err(refusal) => {
                    done.stopped = Some(RelinkStop {
                        path: Some(file.as_str().to_owned()),
                        refusal,
                    });
                    break;
                }
            }
        }
        if let Some(stop) = done.stopped.take() {
            if done.written.is_empty() {
                return (Err(stop.refusal), touched, RenameEnd::Stop);
            }
            done.stopped = Some(stop);
            dedup_pairs(&mut done.fixed);
            return (Ok(done), touched, RenameEnd::Stop);
        }
        // ADR-0400 decision 18: raised before the agents pass, which takes the registry lock an `agent.new` insert
        //   compares the number under — an insert before this raise is in the registry read below, one after it
        //   meets the raised number.
        raise();
        let renames = agent_renames(&self.values.registry.attachments(), from, to);
        if !renames.is_empty() {
            // An agent attached in `from` after the port check above: the cell is asked again.
            let port = match port {
                Some(port) => Ok(port),
                None => self.profile_edit(),
            };
            let rewritten =
                port.and_then(|port| port.rewrite_preset_paths(&renames).map_err(port_refused));
            match rewritten {
                Ok(changed) => {
                    tracing::info!(
                        agents = changed.len(),
                        "agent presets re-pointed by a mount rename"
                    );
                    done.fixed
                        .extend(renames.into_iter().map(|rename| (rename.from, rename.to)));
                }
                Err(refusal) => {
                    done.stopped = Some(RelinkStop {
                        path: None,
                        refusal,
                    });
                    dedup_pairs(&mut done.fixed);
                    return (Ok(done), touched, RenameEnd::Stop);
                }
            }
        }
        dedup_pairs(&mut done.fixed);
        (Ok(done), touched, RenameEnd::Finish)
    }

    /// Whether the mount holding `file` — a catalog file that read as not found inside the hold — is gone itself: its
    /// folder stat'ed fresh (one bounded ask on a new read budget; the command's may be spent) and not `Ok` →
    /// `Unavailable` · `library-unavailable` naming the mount, and the mount, its stat and the sequence drawn before
    /// the stat left in `lost`. `None` = the folder is there (the file was deleted outside Engram); that mount is noted
    /// in `present` and not asked again.
    fn mount_gone(
        &self,
        mounts: &MountsNow,
        file: &LibPath,
        seq: &dyn Fn() -> u64,
        present: &mut HashSet<PathKey>,
        lost: &mut Option<(Mount, FolderStatus, u64)>,
    ) -> Option<LibraryRefusal> {
        let (mount, _) = mounts.locate(file)?;
        if present.contains(&mount.name.key()) {
            return None;
        }
        let at = seq();
        let status = self
            .stat_mounts(&self.io.begin_read(), std::slice::from_ref(&mount.folder))
            .remove(0);
        if status == FolderStatus::Ok {
            present.insert(mount.name.key());
            return None;
        }
        let refusal = LibraryRefusal::Refused {
            class: RefusalClass::Unavailable,
            reason: mount_unavailable(&mount.name, &MountState::of(&status)),
        };
        *lost = Some((mount.clone(), status, at));
        Some(refusal)
    }

    /// The plan of renaming `from` to `to` (ADR-0400 decision 12): the files to rewrite per mount (with whether the
    /// mount's folder is in a git working tree — asked on `read`, outside every lock), the mounts that cannot be read,
    /// the agents to change and what already points at `to`, with the root generation read in the same read lock.
    pub(crate) fn rename_plan(
        &self,
        read: &LaunchIo,
        from: &MountName,
        to: &MountName,
    ) -> RenamePlan {
        let (grouped, unread, plan_digest, agents, root_generation) =
            self.gate.with_read(|state| {
                let sides = Sides::of(&state.catalog, &state.mounts_now(), from, to);
                let files_in = |files: &[LibPath], mount: &Mount| -> Vec<String> {
                    files
                        .iter()
                        .filter(|file| in_mount(file, &mount.name))
                        .map(|file| file.as_str().to_owned())
                        .collect()
                };
                let grouped: Vec<(Mount, Vec<String>)> = state
                    .mounts
                    .iter()
                    .map(|mount| (mount.clone(), files_in(&sides.rewrite, mount)))
                    .filter(|(_, paths)| !paths.is_empty())
                    .collect();
                // ADR-0400 decision 12: a mount that cannot be read is named under `unread` with its files — they are not
                //   rewritten.
                let unread: Vec<PlanUnread> = sides
                    .unusable
                    .iter()
                    .map(|(mount, seen)| {
                        let paths = files_in(&sides.unread, mount);
                        PlanUnread {
                            mount: mount.name.as_str().to_owned(),
                            state: seen.shown(),
                            paths: (seen.walked || !paths.is_empty()).then_some(paths),
                        }
                    })
                    .collect();
                let agents: Vec<(AgentId, String)> = self
                    .values
                    .registry
                    .list()
                    .into_iter()
                    .filter(|profile| {
                        profile
                            .presets
                            .iter()
                            .any(|attached| moved(&attached.path, from, to).is_some())
                    })
                    .map(|profile| (profile.id, profile.canonical_name_when_live()))
                    .collect();
                (
                    grouped,
                    unread,
                    sides.digest(),
                    agents,
                    self.gate.numbers().root,
                )
            });
        let folders: Vec<PathBuf> = grouped
            .iter()
            .map(|(mount, _)| mount.folder.clone())
            .collect();
        let git = self.git_trees(read, &folders);
        let mut agents = agents;
        agents.sort_by(|a, b| a.0.to_string().cmp(&b.0.to_string()));
        RenamePlan {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
            files: grouped
                .into_iter()
                .zip(git)
                .map(|((mount, paths), git)| PlanMount {
                    mount: mount.name.as_str().to_owned(),
                    git,
                    paths,
                })
                .collect(),
            unread,
            agents,
            already_pointing: self.newly_pointing(from, to),
            root_generation,
            plan_digest,
        }
    }

    /// What already points at `to` and will resolve into the renamed mount: nothing when `from` and `to` are the same
    /// name by path key (a case-only rename where keys ignore case — every such reference resolved into it already),
    /// otherwise [`LibraryService::pointing_at`] `to`.
    fn newly_pointing(&self, from: &MountName, to: &MountName) -> Pointing {
        match from.key() == to.key() {
            true => Pointing::default(),
            false => self.pointing_at(to),
        }
    }

    /// Whether each of `folders` lies in a git working tree ([`LibraryService::git_tops`]): `Some(true)` in one,
    /// `Some(false)` in none, `None` = could not be told.
    fn git_trees(&self, read: &LaunchIo, folders: &[PathBuf]) -> Vec<Option<bool>> {
        self.git_tops(read, folders)
            .into_iter()
            .map(|tree| match tree {
                GitTree::At(_) => Some(true),
                GitTree::Outside => Some(false),
                GitTree::Unknown => None,
            })
            .collect()
    }

    /// The git working tree each of `folders` lies in: the nearest `.git` entry from the folder up through its parents
    /// — a folder, or a file (a worktree or submodule) — names it; none found `Outside`; a stat that gave no answer
    /// before a `.git` was found `Unknown` (ADR-0400 decision 24). One bounded batch on `read`.
    pub(crate) fn git_tops(&self, read: &LaunchIo, folders: &[PathBuf]) -> Vec<GitTree> {
        let mut asks: Vec<(PathBuf, Need)> = Vec::new();
        let mut asked: HashSet<PathKey> = HashSet::new();
        for dir in folders.iter().flat_map(|folder| folder.ancestors()) {
            let path = dir.join(".git");
            if asked.insert(path_key(&path)) {
                asks.push((path, Need::Required));
            }
        }
        if asks.is_empty() {
            return Vec::new();
        }
        let statuses = self.io.stat_all(read, &asks);
        folders
            .iter()
            .map(|folder| {
                for dir in folder.ancestors() {
                    match statuses.get(&path_key(&dir.join(".git"))) {
                        Some(FolderStatus::Ok | FolderStatus::NotADirectory) => {
                            return GitTree::At(dir.to_path_buf())
                        }
                        Some(FolderStatus::Missing) => continue,
                        _ => return GitTree::Unknown,
                    }
                }
                GitTree::Outside
            })
            .collect()
    }
}

/// The git working tree a folder lies in ([`LibraryService::git_tops`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitTree {
    /// The folder holding the nearest `.git` (a folder, or a file — a worktree or submodule).
    At(PathBuf),
    /// No `.git` from the folder up.
    Outside,
    /// A stat gave no answer before a `.git` was found.
    Unknown,
}

/// The preset files of the catalog that list a reference the rename respells — its first name is `from` (by path
/// key) and not spelled as `to` — by path.
fn referencing_files(catalog: &Catalog, from: &MountName, to: &MountName) -> Vec<LibPath> {
    let respelled =
        |path: &LibPath| moved_to(path, from, to).is_some_and(|new| new.as_str() != path.as_str());
    let mut files: Vec<LibPath> = catalog
        .items()
        .filter(|item| match &item.content {
            ItemContent::Preset { doc, .. } => preset_refs(doc).any(respelled),
            _ => false,
        })
        .map(|item| item.path.clone())
        .collect();
    files.sort();
    files
}

/// What a rename confirmation covers, as one digest: the preset files to `rewrite`, the referencing files left
/// `unread` in mounts that cannot be read, and the mounts never walked (`unknown`) — one line per entry, `<side>\t<path
/// or mount name>` (a tab is never in a name), sorted, each followed by a newline, then SHA-256 (hex). The same for the
/// same sides whatever the order they are listed in; a file moving between sides changes it.
pub(crate) fn confirmation_digest<'a>(
    rewrite: impl Iterator<Item = &'a str>,
    unread: impl Iterator<Item = &'a str>,
    unknown: impl Iterator<Item = &'a str>,
) -> String {
    lines_digest(
        rewrite
            .map(|path| format!("rewrite\t{path}"))
            .chain(unread.map(|path| format!("unread\t{path}")))
            .chain(unknown.map(|mount| format!("unknown\t{mount}")))
            .collect(),
    )
}

/// The digest of a confirmation's `lines` (each `<side>\t<entry>`, a tab never in a name): sorted, each followed by a
/// newline, then SHA-256 (hex) — the same whatever order they come in.
pub(crate) fn lines_digest(mut lines: Vec<String>) -> String {
    lines.sort_unstable();
    let mut text = String::new();
    for line in lines {
        text.push_str(&line);
        text.push('\n');
    }
    sha256_hex(text.as_bytes())
}

/// Whether `path`'s first name is `from` by path key.
fn in_mount(path: &LibPath, from: &MountName) -> bool {
    let first = path.as_str().split('/').next().unwrap_or_default();
    path_key(Path::new(first)) == from.key()
}

/// A stored spelling moved from `from` to `to` — `None` = not a library path in `from`, or already spelled so.
fn moved(spelling: &str, from: &MountName, to: &MountName) -> Option<String> {
    match AssetRef::parse(spelling) {
        Ok(AssetRef::Library(path)) => moved_to(&path, from, to)
            .map(|new| new.as_str().to_owned())
            .filter(|new| new != spelling),
        _ => None,
    }
}

/// Whether an agent attaches something in `from`.
fn agents_in(attachments: &[(AgentId, Attachments)], from: &MountName) -> bool {
    attachments.iter().any(|(_, attached)| {
        attached.presets.iter().any(|preset| {
            matches!(AssetRef::parse(&preset.path), Ok(AssetRef::Library(path)) if in_mount(&path, from))
        })
    })
}

/// One rename line per attached spelling in `from` (each once, by platform `same_path`) — the port matches whole paths.
fn agent_renames(
    attachments: &[(AgentId, Attachments)],
    from: &MountName,
    to: &MountName,
) -> Vec<PresetPathRename> {
    let mut renames: Vec<PresetPathRename> = Vec::new();
    for (_, attached) in attachments {
        for preset in &attached.presets {
            let Some(new) = moved(&preset.path, from, to) else {
                continue;
            };
            if renames
                .iter()
                .any(|rename| same_path(Path::new(&rename.from), Path::new(&preset.path)))
            {
                continue;
            }
            renames.push(PresetPathRename {
                from: preset.path.clone(),
                to: new,
            });
        }
    }
    renames
}

/// The lists of `doc` with every library entry in `from` re-pointed at `to` (its origin and tag kept); in a list that
/// changed only the first place of each target stays (ADR-0339). Answers the lists and the pairs re-pointed.
fn moved_refs(
    doc: &PresetDoc,
    from: &MountName,
    to: &MountName,
) -> (PresetLists, Vec<(String, String)>) {
    let mut lists = as_read(doc);
    let mut pairs = Vec::new();
    for list in ITEM_LISTS {
        let entries = list_mut(&mut lists, list);
        let mut changed = false;
        for entry in entries.iter_mut() {
            let AssetRef::Library(path) = &entry.target else {
                continue;
            };
            let Some(new) = moved_to(path, from, to).filter(|new| new.as_str() != path.as_str())
            else {
                continue;
            };
            pairs.push((path.as_str().to_owned(), new.as_str().to_owned()));
            entry.target = AssetRef::Library(new);
            changed = true;
        }
        if changed {
            // ADR-0339
            let mut seen = HashSet::new();
            entries.retain(|entry| seen.insert(ref_key(&entry.target)));
        }
    }
    (lists, pairs)
}

/// Keeps the first of each pair by its old path's key.
fn dedup_pairs(pairs: &mut Vec<(String, String)>) {
    let mut seen = HashSet::new();
    pairs.retain(|(from, _)| seen.insert(path_key(Path::new(from))));
}

/// The `why` head of a rename whose save failed.
fn rename_unsaved(from: &MountName, to: &MountName) -> String {
    format!(
        "the library mount {} renamed to {} is live but not saved",
        from.as_str(),
        to.as_str()
    )
}

/// `Conflict` · [`MOUNT_UNUSABLE`] `{mount, state, why}`.
fn mount_unusable(name: &MountName, state: &MountState) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        MOUNT_UNUSABLE,
        json!({ "mount": name.as_str(), "state": state.word(), "why": state.why() }),
    )
}

/// `Conflict` · [`MOUNT_RENAME_UNCONFIRMED`] `{plan, root_generation, plan_digest}`.
fn unconfirmed(plan: &RenamePlan) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        MOUNT_RENAME_UNCONFIRMED,
        json!({
            "plan": plan.to_json(),
            "root_generation": plan.root_generation,
            "plan_digest": plan.digest(),
        }),
    )
}

/// `Conflict` · `root-generation-stale` `{given, current, plan, plan_digest}` — with a fresh plan.
fn stale_with_plan(given: u64, plan: &RenamePlan) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        ROOT_GENERATION_STALE,
        json!({
            "given": given,
            "current": plan.root_generation,
            "plan": plan.to_json(),
            "plan_digest": plan.digest(),
        }),
    )
}
