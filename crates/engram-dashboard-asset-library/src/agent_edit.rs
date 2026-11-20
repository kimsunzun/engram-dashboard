//! Agent-preset view and edits: the `agent.presets` view, the attachment edits (`attach` · `detach` · `reorder`) and
//! the `agent.new {presets}` judgement — inherent [`LibraryService`] methods answering crate-local types.
//!
//! ## Rules (invariants)
//!
//! - **Writes go only through the port, inside one change.** An edit never mutates `ProfileRegistry`: inside one
//!   [`LibraryService::change_at`] it reads the agent from the registry, judges the request against the catalog and
//!   writes the whole new attachments with `ProfileEditPort::set_attachments`. The port's last-write-wins contract
//!   holds only because every writer of an existing agent's attachments queues under the library write lock.
//! - **The port cell must be filled.** An empty or dead cell refuses `Internal` · [`PROFILE_EDIT_UNAVAILABLE`] before
//!   anything is read or written — never a silent no-op.
//! - **An agent takes presets only**: a library preset, or `engram:/default` for its Default slot.
//! - **Only an `attach` naming a library preset needs the library.** It refuses `Unavailable` · `library-unavailable`
//!   when no first scan could be applied, nothing applied. `detach`, `reorder` and an `attach` of `engram:/default`
//!   alone judge nothing against the catalog and work with no root; their answer view shows the rows resolution gives
//!   under that root state.
//! - **Answers are built inside the change.** The answer view is resolved inside the change from the attachments as
//!   committed; its folder stats and link probes run after it, outside every lock.
//! - **`agent.new {presets}` is judged, not edited.** The judgement is `attach`'s over the profile to be created
//!   (outside the registry), under one read lock that also compares the root generation; it writes nothing and needs
//!   no port — the caller creates the profile with the accepted list in one registry write (ADR-0386 decision 5).
//!
//! ## Refusals every edit can answer
//!
//! The port cell empty or dead `Internal` · [`PROFILE_EDIT_UNAVAILABLE`]; a stale root generation `Conflict` ·
//! `root-generation-stale`; an agent not in the registry `NotFound` · `not-found` (`args` = `{agent}`); the port's
//! store error passed through unchanged ([`LibraryRefusal::Store`]), nothing changed.
// ADR-0378
// ADR-0379
// ADR-0380

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use engram_dashboard_agent::launch::{backend_word, BlockReason, Notice, Reason};
use engram_dashboard_agent::profile::{AgentOutputFormat, AgentProfile, AttachedPreset};
use engram_dashboard_agent::profile_edit::{
    Attachments, ProfileEditError, ProfileEditPort, WorkspaceLock,
};
use engram_dashboard_agent::types::AgentId;
use serde_json::json;

use crate::catalog::Catalog;
use crate::edit::{
    list_of, parse_ref, skip, Skipped, ALREADY_PRESENT, REORDER_ORDER_MISMATCH, REV_MISMATCH,
};
use crate::folder::FolderStatus;
use crate::format::read::WRONG_KIND;
use crate::format::ListName;
use crate::gate::NOT_FOUND;
use crate::launch::sha256_hex;
use crate::path::{AssetRef, BuiltinId};
use crate::resolve::{
    ref_key, resolve, CatalogLookup, Looked, Lookup, Problem, RefKey, Resolution, ResolvedFolder,
    RootState, Target,
};
use crate::{output_format, stale_root_generation, LibraryRefusal, LibraryService, RefusalClass};

/// The code of a `detach` of `engram:/default` without `yes` while the agent's Default slot is on (`args` =
/// `{agent}`).
pub const DEFAULT_PRESET_DETACH: &str = "default-preset-detach";

/// The code of an agent-preset body while the profile-edit port is not plugged in or its manager is gone — a wiring bug
/// (`args` = `{}`).
// ADR-0380
pub const PROFILE_EDIT_UNAVAILABLE: &str = "profile-edit-unavailable";

/// The `agent.presets` view.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentPresetsView {
    pub agent: AgentId,
    /// The Default slot (`engram:/default`).
    pub default_preset: bool,
    /// The attached presets as the profile spells them, in attach order.
    pub presets: Vec<String>,
    /// The working folder as the profile stores it.
    pub cwd: PathBuf,
    /// Why the working folder is locked, as the port answers; `None` = it can be changed.
    pub workspace_lock: Option<WorkspaceLock>,
    /// The agent's backend word and mode — agent values, never changed here.
    pub backend: String,
    pub mode: AgentOutputFormat,
    /// What a `reorder` must carry: the SHA-256 of the compact canonical JSON `{"default":<bool>,"presets":[<paths in
    /// order>]}` (ADR-0378).
    pub base_rev: String,
    /// `false` = its backend has no settings check: it takes no preset, and the rows below stay empty.
    pub takes_presets: bool,
    /// The folder entries its presets reach, in expansion order, each with its stat.
    pub folders: Vec<ResolvedFolder>,
    pub conflicts: Vec<Problem>,
    /// Every other problem row, inner presets' with their `via` — a launch leaves out the `missing` rows (and the
    /// library row of an unset or absent root) and refuses on the others.
    pub problems: Vec<Problem>,
    pub notices: Vec<Notice>,
}

/// The answer of [`LibraryService::attach_preset`]: what was attached, what was skipped and why, and the view after
/// the edit.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentAttach {
    pub applied: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub view: AgentPresetsView,
}

/// The `agent.new {presets}` judgement: the attachments to create the agent with, and what was skipped and why.
#[derive(Clone, Debug, PartialEq)]
pub struct NewPresets {
    pub attachments: Attachments,
    pub skipped: Vec<Skipped>,
}

/// The answer view as built inside the change; the folder stats and link probes are still to run.
struct PendingAgentView {
    profile: AgentProfile,
    workspace_lock: Option<WorkspaceLock>,
    resolution: Option<Resolution>,
    root: Option<PathBuf>,
}

/// The first-scan entrance's answer and the root stat an edit took before its change, outside every lock.
struct RootSeen {
    joined: Result<(), String>,
    /// The root the stat was taken of.
    seen: Option<PathBuf>,
    /// Taken whenever a root is set, the first scan failed or not.
    stat: Option<FolderStatus>,
}

impl RootSeen {
    /// The root state the answer view resolves under, for `root` as the change found it — the rule of the read path
    /// (`LibraryService::root_state`, [`RootState::of`]), so the edit's answer equals `agent.presets` (ADR-0384). A
    /// root published after the stat counts as there: its publish stat'ed it.
    // ADR-0387
    fn state(&self, root: Option<&Path>, attached_library: bool) -> RootState {
        let Some(now) = root else {
            return RootState::Unset;
        };
        let stat = match (&self.seen, &self.stat) {
            _ if !attached_library => None,
            (Some(seen), Some(stat)) if seen == now => Some(stat.clone()),
            _ => Some(FolderStatus::Ok),
        };
        RootState::of(&self.joined, stat.as_ref())
    }
}

/// An attached path as resolution identifies it; a spelling that is not a reference only by its text.
#[derive(Clone, PartialEq, Eq, Hash)]
enum AttachedKey {
    Ref(RefKey),
    Raw(String),
}

impl LibraryService {
    /// The `agent.presets` view of `agent`: its attachments, working folder lock (through the port), backend, mode and
    /// `baseRev`, and the folder, conflict, problem and notice rows of what it gets. Resolution passes the first-scan
    /// entrance; with no first scan applied the rows are what resolution gives under that root state.
    ///
    /// Refusals: the port cell empty or dead `Internal` · [`PROFILE_EDIT_UNAVAILABLE`]; an agent not in the registry
    /// `NotFound` · `not-found`.
    pub fn agent_presets(&self, agent: AgentId) -> Result<AgentPresetsView, LibraryRefusal> {
        let port = self.profile_edit()?;
        let profile = self
            .values
            .registry
            .get(agent)
            .ok_or_else(|| agent_not_found(agent))?;
        let resolution = self
            .want_of(&profile)
            .map(|want| self.resolve_wants(&[&want]).remove(0));
        let workspace_lock = port.workspace_lock(agent).map_err(port_refused)?;
        Ok(view_of(&profile, workspace_lock, resolution))
    }

    /// Attaches `presets` to `agent` (partial apply): a library preset goes into the list right after the `after`
    /// anchor in call order (no anchor in the list = at its end); `engram:/default` turns the Default slot on. Each one
    /// that cannot be attached is skipped with a reason: not a reference (`item-path-invalid`), not in the catalog
    /// (`not-found`), not a preset (`wrong-kind`), already attached ([`ALREADY_PRESENT`], by catalog key). Naming a
    /// library preset with no first scan applied refuses `Unavailable` · `library-unavailable`, nothing applied. Plus
    /// the refusals every edit can answer (module header).
    pub fn attach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<AgentAttach, LibraryRefusal> {
        let library = names_library(presets);
        let ((applied, skipped), view) =
            self.agent_edit(agent, root_generation, library, |profile, lookup| {
                Ok(attach(profile, lookup, presets, after))
            })?;
        Ok(AgentAttach {
            applied,
            skipped,
            view,
        })
    }

    /// Detaches `presets` from `agent`: every attached entry with the same catalog key goes; `engram:/default` turns
    /// the Default slot off. A spelling that is not a reference matches an attached entry by its exact text, so a
    /// stored entry that does not parse can be dropped. Idempotent for references (a repeated raw spelling refuses once its
    /// entry is gone) and not edition-checked. A spelling that is not a
    /// reference and matches no attached entry refuses the whole call `InvalidArgument` · `item-path-invalid`;
    /// `engram:/default` while the slot is on and `yes` is not given refuses `Conflict` · [`DEFAULT_PRESET_DETACH`].
    /// Plus the refusals every edit can answer (module header).
    pub fn detach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        yes: bool,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsView, LibraryRefusal> {
        let mut default_named = false;
        let mut keys: Vec<AttachedKey> = Vec::with_capacity(presets.len());
        // A spelling that does not parse, with the refusal it answers if no attached entry carries its text.
        let mut unparsed: Vec<(&str, Reason)> = Vec::new();
        for preset in presets {
            match parse_ref(preset) {
                Ok(target) => {
                    default_named |= matches!(target, AssetRef::BuiltIn(BuiltinId::DefaultPreset));
                    keys.push(AttachedKey::Ref(ref_key(&target)));
                }
                Err(reason) => {
                    keys.push(AttachedKey::Raw(preset.clone()));
                    unparsed.push((preset, reason));
                }
            }
        }
        let ((), view) = self.agent_edit(agent, root_generation, false, |profile, _| {
            if let Some((_, reason)) = unparsed.into_iter().find(|(text, _)| {
                !profile
                    .presets
                    .iter()
                    .any(|attached| attached.path == *text)
            }) {
                return Err(LibraryRefusal::Refused {
                    class: RefusalClass::InvalidArgument,
                    reason,
                });
            }
            if default_named && profile.default_preset && !yes {
                return Err(LibraryRefusal::new(
                    RefusalClass::Conflict,
                    DEFAULT_PRESET_DETACH,
                    json!({ "agent": agent.to_string() }),
                ));
            }
            let next = Attachments {
                default_preset: profile.default_preset && !default_named,
                presets: profile
                    .presets
                    .iter()
                    .filter(|attached| !keys.contains(&attached_key(&attached.path)))
                    .cloned()
                    .collect(),
            };
            Ok((next, ()))
        })?;
        Ok(view)
    }

    /// Reorders `agent`'s attached presets to `order`, which must be a permutation of the list as the view shows it
    /// (by catalog key, a duplicate counted as often as attached) — else `InvalidArgument` ·
    /// [`REORDER_ORDER_MISMATCH`]. A `base_rev` other than the agent's `baseRev` computed from the registry read inside
    /// the change refuses `Conflict` · [`REV_MISMATCH`]. The Default slot is untouched. Plus the refusals every edit can
    /// answer (module header).
    pub fn reorder_presets(
        &self,
        agent: AgentId,
        order: &[String],
        base_rev: &str,
        root_generation: Option<u64>,
    ) -> Result<AgentPresetsView, LibraryRefusal> {
        let ((), view) = self.agent_edit(agent, root_generation, false, |profile, _| {
            let current = agent_base_rev(profile.default_preset, &profile.presets);
            if base_rev != current {
                return Err(LibraryRefusal::new(
                    RefusalClass::Conflict,
                    REV_MISMATCH,
                    json!({ "rev": current }),
                ));
            }
            let next = Attachments {
                default_preset: profile.default_preset,
                presets: reorder(&profile.presets, order)?,
            };
            Ok((next, ()))
        })?;
        Ok(view)
    }

    /// The `agent.new {presets}` judgement: `presets` judged over `profile` — the profile to be created, not in the
    /// registry — exactly as [`LibraryService::attach_preset`] judges them (no anchor: in call order at the end), under
    /// one read lock. Nothing is written and no port is needed. Naming a library preset with no first scan applied
    /// refuses `Unavailable` · `library-unavailable`; a stale `root_generation`, compared inside the read lock,
    /// refuses `Conflict` · `root-generation-stale`.
    // ADR-0386
    pub fn judge_new_presets(
        &self,
        profile: &AgentProfile,
        presets: &[String],
        root_generation: Option<u64>,
    ) -> Result<NewPresets, LibraryRefusal> {
        // Only a library preset is looked up in the catalog, so only then is the first scan waited for.
        if names_library(presets) {
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
        }
        self.gate.with_read(|state| {
            let current = self.gate.numbers().root;
            if root_generation.is_some_and(|given| given != current) {
                return Err(stale_root_generation(root_generation, current));
            }
            let lookup = CatalogLookup {
                catalog: &state.catalog,
                root: state.root.as_deref(),
                builtins: &self.builtins,
            };
            let (attachments, (_, skipped)) = attach(profile, &lookup, presets, None);
            Ok(NewPresets {
                attachments,
                skipped,
            })
        })
    }

    /// The live profile-edit port, or `Internal` · [`PROFILE_EDIT_UNAVAILABLE`] (empty or dead cell).
    // ADR-0380
    pub(crate) fn profile_edit(
        &self,
    ) -> Result<std::sync::Arc<dyn ProfileEditPort>, LibraryRefusal> {
        self.profile_edit
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                LibraryRefusal::new(RefusalClass::Internal, PROFILE_EDIT_UNAVAILABLE, json!({}))
            })
    }

    /// One agent-preset edit (module header): the port, the first-scan entrance (refusing only when `library` — the
    /// edit judges library presets against the catalog), the root stat for the answer view, then one
    /// [`LibraryService::change_at`] ([`LibraryService::agent_edit_in`]); the answer view is settled after it.
    fn agent_edit<T>(
        &self,
        agent: AgentId,
        root_generation: Option<u64>,
        library: bool,
        op: impl FnOnce(&AgentProfile, &CatalogLookup<'_>) -> Result<(Attachments, T), LibraryRefusal>,
    ) -> Result<(T, AgentPresetsView), LibraryRefusal> {
        let port = self.profile_edit()?;
        let joined = self.first_scan_join();
        if let (true, Err(why)) = (library, &joined) {
            return Err(LibraryRefusal::unavailable(why.clone()));
        }
        // The stat runs here, outside every lock.
        let seen = self.gate.with_read(|state| state.root.clone());
        let stat = seen
            .as_deref()
            .map(|root| self.stat_root(&self.io.begin_read(), root));
        let before = RootSeen { joined, seen, stat };
        let (answer, pending) = self.change_at(root_generation, |catalog, root, _| {
            let edited = self.agent_edit_in(&*port, agent, catalog, root, &before, op);
            (edited, Vec::new())
        })??;
        Ok((answer, self.settle_agent(pending)))
    }

    /// The steps of [`LibraryService::agent_edit`] inside the change: the registry read, `op` over the profile and the
    /// catalog, the port's lock read and write, and the answer view of the attachments as committed.
    fn agent_edit_in<T>(
        &self,
        port: &dyn ProfileEditPort,
        agent: AgentId,
        catalog: &Catalog,
        root: Option<&Path>,
        before: &RootSeen,
        op: impl FnOnce(&AgentProfile, &CatalogLookup<'_>) -> Result<(Attachments, T), LibraryRefusal>,
    ) -> Result<(T, PendingAgentView), LibraryRefusal> {
        let lookup = CatalogLookup {
            catalog,
            root,
            builtins: &self.builtins,
        };
        let profile = self
            .values
            .registry
            .get(agent)
            .ok_or_else(|| agent_not_found(agent))?;
        let (next, answer) = op(&profile, &lookup)?;
        let workspace_lock = port.workspace_lock(agent).map_err(port_refused)?;
        port.set_attachments(agent, next.clone())
            .map_err(port_refused)?;
        tracing::info!(agent = %agent, "agent presets set");
        let committed = AgentProfile {
            default_preset: next.default_preset,
            presets: next.presets,
            ..profile
        };
        let resolution = self.want_of(&committed).map(|want| {
            let attached_library = want
                .presets
                .iter()
                .any(|preset| matches!(preset, AssetRef::Library(_)));
            let target = Target {
                backend: want.backend,
                mode: want.mode,
                default_preset: want.default_preset,
                presets: want.presets,
                unparsed: want.unparsed,
                root: before.state(root, attached_library),
            };
            resolve(&target, &lookup, &*self.values.backends)
        });
        let pending = PendingAgentView {
            profile: committed,
            workspace_lock,
            resolution,
            root: root.map(Path::to_path_buf),
        };
        Ok((answer, pending))
    }

    /// Finishes an edit's answer view outside every lock.
    fn settle_agent(&self, pending: PendingAgentView) -> AgentPresetsView {
        let PendingAgentView {
            profile,
            workspace_lock,
            mut resolution,
            root,
        } = pending;
        if let Some(resolution) = &mut resolution {
            self.settle_resolution(resolution, root.as_deref());
        }
        view_of(&profile, workspace_lock, resolution)
    }
}

/// The view of `profile` with its settled resolution (`None` = it takes no preset).
fn view_of(
    profile: &AgentProfile,
    workspace_lock: Option<WorkspaceLock>,
    resolution: Option<Resolution>,
) -> AgentPresetsView {
    let (takes_presets, folders, conflicts, problems, notices) = match resolution {
        Some(resolution) => {
            let (conflicts, problems) = resolution
                .problems
                .into_iter()
                .partition(|problem| matches!(problem, BlockReason::Conflict { .. }));
            (
                true,
                resolution.folders,
                conflicts,
                problems,
                resolution.notices,
            )
        }
        None => (false, Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };
    AgentPresetsView {
        agent: profile.id,
        default_preset: profile.default_preset,
        presets: profile
            .presets
            .iter()
            .map(|attached| attached.path.clone())
            .collect(),
        cwd: profile.cwd.clone(),
        workspace_lock,
        backend: backend_word(&profile.command),
        mode: output_format(&profile.command),
        base_rev: agent_base_rev(profile.default_preset, &profile.presets),
        takes_presets,
        folders,
        conflicts,
        problems,
        notices,
    }
}

/// The `agent.new {presets}` judgement where no library service runs (the daemon's `NoLibrary`): the attach rule with
/// no catalog — a library preset refuses the whole call `Unavailable` · `library-unavailable` (`message` = `why`), and
/// the rest is judged as [`LibraryService::judge_new_presets`] judges it. No root generation is compared: there is no
/// root.
// ADR-0386
pub fn judge_new_presets_without_library(
    profile: &AgentProfile,
    presets: &[String],
    why: &str,
) -> Result<NewPresets, LibraryRefusal> {
    if names_library(presets) {
        return Err(LibraryRefusal::unavailable(why.to_owned()));
    }
    let (attachments, (_, skipped)) = attach(profile, &NoCatalog, presets, None);
    Ok(NewPresets {
        attachments,
        skipped,
    })
}

/// A library that holds nothing — the lookup of a judgement with no library service, which refuses a library preset
/// before anything is looked up.
struct NoCatalog;

impl Lookup for NoCatalog {
    fn item(&self, _: &AssetRef) -> Looked<'_> {
        Looked::Missing
    }
}

/// Whether `presets` name a library preset — the one case an attach needs the library for.
fn names_library(presets: &[String]) -> bool {
    presets
        .iter()
        .any(|preset| matches!(AssetRef::parse(preset), Ok(AssetRef::Library(_))))
}

/// The attachments `presets` make over `profile`, and what was applied and skipped (see
/// [`LibraryService::attach_preset`]).
fn attach(
    profile: &AgentProfile,
    lookup: &dyn Lookup,
    presets: &[String],
    after: Option<&str>,
) -> (Attachments, (Vec<String>, Vec<Skipped>)) {
    let mut next = Attachments {
        default_preset: profile.default_preset,
        presets: profile.presets.clone(),
    };
    let anchor = after.map(attached_key);
    let mut at: Option<usize> = None;
    let mut keys: HashSet<AttachedKey> = next
        .presets
        .iter()
        .map(|attached| attached_key(&attached.path))
        .collect();
    let (mut applied, mut skipped) = (Vec::new(), Vec::new());
    for preset in presets {
        let target = match parse_ref(preset) {
            Ok(target) => target,
            Err(reason) => {
                skipped.push(Skipped {
                    item: preset.clone(),
                    reason,
                });
                continue;
            }
        };
        match &target {
            AssetRef::BuiltIn(BuiltinId::DefaultPreset) => {
                if next.default_preset {
                    skipped.push(skip(preset, ALREADY_PRESENT, None));
                } else {
                    next.default_preset = true;
                    applied.push(preset.clone());
                }
                continue;
            }
            AssetRef::BuiltIn(_) => {
                skipped.push(skip(preset, WRONG_KIND, None));
                continue;
            }
            AssetRef::Library(_) => {}
        }
        match lookup.item(&target) {
            Looked::Present(view) if list_of(view.kind) == Some(ListName::Presets) => {}
            Looked::Present(_) => {
                skipped.push(skip(preset, WRONG_KIND, None));
                continue;
            }
            Looked::Missing | Looked::Unreadable(_) => {
                skipped.push(skip(preset, NOT_FOUND, None));
                continue;
            }
        }
        let key = AttachedKey::Ref(ref_key(&target));
        if !keys.insert(key) {
            skipped.push(skip(preset, ALREADY_PRESENT, None));
            continue;
        }
        let place = at.or_else(|| {
            let anchor = anchor.as_ref()?;
            let found = next
                .presets
                .iter()
                .position(|attached| attached_key(&attached.path) == *anchor)?;
            Some(found + 1)
        });
        let new = AttachedPreset {
            path: target.spelling(),
        };
        match place {
            Some(place) => {
                next.presets.insert(place, new);
                at = Some(place + 1);
            }
            None => next.presets.push(new),
        }
        applied.push(preset.clone());
    }
    (next, (applied, skipped))
}

/// `current` in `order` (matched by [`attached_key`], each entry once), or [`REORDER_ORDER_MISMATCH`].
fn reorder(
    current: &[AttachedPreset],
    order: &[String],
) -> Result<Vec<AttachedPreset>, LibraryRefusal> {
    let mismatch = || {
        let expected: Vec<&str> = current
            .iter()
            .map(|attached| attached.path.as_str())
            .collect();
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            REORDER_ORDER_MISMATCH,
            json!({ "expected": expected }),
        )
    };
    if order.len() != current.len() {
        return Err(mismatch());
    }
    let mut used = vec![false; current.len()];
    let mut reordered = Vec::with_capacity(current.len());
    for spelling in order {
        let wanted = attached_key(spelling);
        let at = (0..current.len())
            .find(|&at| !used[at] && attached_key(&current[at].path) == wanted)
            .ok_or_else(mismatch)?;
        used[at] = true;
        reordered.push(current[at].clone());
    }
    Ok(reordered)
}

fn attached_key(path: &str) -> AttachedKey {
    match AssetRef::parse(path) {
        Ok(target) => AttachedKey::Ref(ref_key(&target)),
        Err(_) => AttachedKey::Raw(path.to_owned()),
    }
}

/// The agent `baseRev` (ADR-0378): lowercase-hex SHA-256 of `{"default":<bool>,"presets":[<paths in order>]}`.
fn agent_base_rev(default_preset: bool, presets: &[AttachedPreset]) -> String {
    let paths: Vec<&str> = presets
        .iter()
        .map(|attached| attached.path.as_str())
        .collect();
    let canonical = json!({ "default": default_preset, "presets": paths }).to_string();
    sha256_hex(canonical.as_bytes())
}

/// A port refusal: an agent gone from the registry `NotFound` · `not-found`; a store error passed through unchanged.
pub(crate) fn port_refused(error: ProfileEditError) -> LibraryRefusal {
    match error {
        ProfileEditError::NotFound(agent) => agent_not_found(agent),
        ProfileEditError::Store(store) => LibraryRefusal::Store(store),
    }
}

fn agent_not_found(agent: AgentId) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::NotFound,
        NOT_FOUND,
        json!({ "agent": agent.to_string() }),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
    use std::thread;
    use std::time::Duration;

    use engram_dashboard_agent::commands::store_command_error;
    use engram_dashboard_agent::profile::{
        AgentCommand, ProfileRegistry, ProfileStore, Refusal, StoreError,
    };
    use engram_dashboard_agent::profile_edit::PresetPathRename;
    use engram_dashboard_base::sync::lock;
    use engram_dashboard_command::{ErrorCode, RetryMode};
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, RootSetting};
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::format::read::ITEM_PATH_INVALID;
    use crate::path::LibPath;
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing, TrashBundle};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE, ROOT_GENERATION_STALE};

    /// [`FsStore`] whose next walk parks when a test arms it: it says it arrived, then waits for the go.
    #[derive(Default)]
    struct Held {
        hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        /// Every scan fails as an unreadable root.
        fail_scans: std::sync::atomic::AtomicBool,
    }

    impl LibraryStore for Held {
        fn scan(
            &self,
            root: &Path,
            under: Option<&LibPath>,
            seq: &dyn Fn() -> u64,
        ) -> std::io::Result<ScanListing> {
            if self.fail_scans.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            let parked = lock(&self.hold).take();
            if let Some((arrived, go)) = parked {
                arrived.send(()).unwrap();
                go.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            FsStore.scan(root, under, seq)
        }
        fn first_link(&self, root: &Path, rel: &LibPath) -> std::io::Result<Option<LibPath>> {
            FsStore.first_link(root, rel)
        }
        fn read(&self, path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
            FsStore.read(path, cap)
        }
        fn tag(&self, path: &Path) -> std::io::Result<ContentTag> {
            FsStore.tag(path)
        }
        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            FsStore.write_atomic(path, bytes)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            FsStore.create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> std::io::Result<()> {
            FsStore.create_dir(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            FsStore.rename(from, to)
        }
        fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            FsStore.copy(from, to)
        }
        fn trash(
            &self,
            root: &Path,
            path: &Path,
            bundle: &TrashBundle,
        ) -> std::io::Result<PathBuf> {
            FsStore.trash(root, path, bundle)
        }
        fn exists(&self, path: &Path) -> bool {
            FsStore.exists(path)
        }
    }

    struct Saves(Vec<AgentProfile>);

    impl ProfileStore for Saves {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    /// A profile-edit port that logs each write with whether the library write lock was held, writes it into the
    /// registry as the manager does (or not, when `applies` is off), fails the next write when a test sets a failure,
    /// and answers a fixed working-folder lock.
    #[derive(Default)]
    struct Port {
        registry: OnceLock<Arc<ProfileRegistry>>,
        applies: bool,
        svc: OnceLock<Weak<LibraryService>>,
        writes: Mutex<Vec<(AgentId, Attachments, bool)>>,
        fail: Mutex<Option<ProfileEditError>>,
        lock: Mutex<Option<WorkspaceLock>>,
    }

    impl Port {
        /// Whether the library write lock is held — a read lock lets `try_read` through, so it does not count.
        fn under_lock(&self) -> bool {
            let svc = self.svc.get().and_then(Weak::upgrade).expect("the service");
            let lib = svc.gate.lib();
            let held = lib.try_write().is_err() && lib.try_read().is_err();
            held
        }

        fn writes(&self) -> Vec<(AgentId, Attachments, bool)> {
            lock(&self.writes).clone()
        }
    }

    impl ProfileEditPort for Port {
        fn set_attachments(&self, id: AgentId, next: Attachments) -> Result<(), ProfileEditError> {
            lock(&self.writes).push((id, next.clone(), self.under_lock()));
            if let Some(failure) = lock(&self.fail).take() {
                return Err(failure);
            }
            if !self.applies {
                return Ok(());
            }
            let registry = self.registry.get().expect("the registry");
            let present = registry.try_update_with(id, |profile| {
                profile.default_preset = next.default_preset;
                profile.presets = next.presets;
            })?;
            match present {
                true => Ok(()),
                false => Err(ProfileEditError::NotFound(id)),
            }
        }

        fn rewrite_preset_paths(
            &self,
            _: &[PresetPathRename],
        ) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not an agent-preset body's call")
        }

        fn remove_preset_paths(&self, _: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
            unreachable!("not an agent-preset body's call")
        }

        fn workspace_lock(&self, id: AgentId) -> Result<Option<WorkspaceLock>, ProfileEditError> {
            match self.registry.get().expect("the registry").get(id) {
                Some(_) => Ok(*lock(&self.lock)),
                None => Err(ProfileEditError::NotFound(id)),
            }
        }
    }

    /// A temp folder holding the library root (published when `root`), a registry over `profiles`, and the port
    /// (plugged when `plugged`).
    struct World {
        _temp: TempDir,
        base: PathBuf,
        registry: Arc<ProfileRegistry>,
        store: Arc<Held>,
        port: Arc<Port>,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new(profiles: Vec<AgentProfile>) -> World {
            World::make(profiles, true, true, true)
        }

        fn make(profiles: Vec<AgentProfile>, root: bool, plugged: bool, applies: bool) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            let state = base.join("state");
            fs::create_dir_all(&state).unwrap();
            let registry = Arc::new(ProfileRegistry::new(Arc::new(Saves(profiles))));
            let config = match root {
                true => LibraryConfig::usable(&state, RootSetting::Explicit(base.join("lib"))),
                false => LibraryConfig::missing(&state),
            };
            let values = LibraryValues {
                registry: Arc::clone(&registry),
                backends: Arc::new(AgentBackends),
                config,
                default_root: base.join("default-library"),
                daemon_id: "test-daemon".to_owned(),
                root_generation_base: 7,
                builtin_dir: None,
                launch_root: base.join("launch"),
                home: None,
                env: Arc::new(|_| None),
            };
            let io = BoundedIo::new(
                IO_LIMIT,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::new(StdFolderOps),
                Arc::new(ThreadSpawn),
            );
            let store = Arc::new(Held::default());
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::clone(&store) as Arc<dyn LibraryStore>,
                io,
            ));
            let port = Arc::new(Port {
                applies,
                ..Port::default()
            });
            let _ = port.registry.set(Arc::clone(&registry));
            let _ = port.svc.set(Arc::downgrade(&svc));
            if plugged {
                let weak: Weak<Port> = Arc::downgrade(&port);
                svc.plug_profile_edit(weak);
            }
            World {
                _temp: temp,
                base,
                registry,
                store,
                port,
                svc,
            }
        }

        fn put(&self, rel: &str, text: &str) {
            let path = self.base.join("lib").join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        /// Presets `P.yaml`, `Q.yaml`, `R.yaml`, `S.yaml` and instruction `rules/a.md`.
        fn library(&self) {
            for name in ["P.yaml", "Q.yaml", "R.yaml", "S.yaml"] {
                self.put(name, "kind: preset\nversion: 1\n");
            }
            self.put("rules/a.md", "a");
        }

        fn attachments(&self, agent: AgentId) -> Attachments {
            self.registry.attachments_of(agent).expect("the agent")
        }
    }

    fn claude(presets: &[&str], default_preset: bool) -> AgentProfile {
        let mut profile = AgentProfile::new(
            "a".to_owned(),
            AgentCommand::Claude {
                extra_args: Vec::new(),
                output_format: AgentOutputFormat::StreamJson,
            },
            PathBuf::from("/work"),
            Vec::new(),
            false,
        );
        profile.default_preset = default_preset;
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    fn attached(default_preset: bool, paths: &[&str]) -> Attachments {
        Attachments {
            default_preset,
            presets: paths
                .iter()
                .map(|path| AttachedPreset {
                    path: (*path).to_owned(),
                })
                .collect(),
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_owned()).collect()
    }

    fn refused<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (RefusalClass, Reason) {
        match answer {
            Err(LibraryRefusal::Refused { class, reason }) => (class, reason),
            other => panic!("expected a refusal: {other:?}"),
        }
    }

    fn code<T: std::fmt::Debug>(answer: Result<T, LibraryRefusal>) -> (RefusalClass, String) {
        let (class, reason) = refused(answer);
        (class, reason.code)
    }

    fn sha256_of(text: &str) -> String {
        sha256_hex(text.as_bytes())
    }

    // ── writes through the port ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn attach_detach_and_reorder_write_only_through_the_port_from_inside_a_change() {
        let profile = claude(&["P.yaml", "Q.yaml"], true);
        let agent = profile.id;
        // The port logs and does not write: whatever reaches the registry did not come through it.
        let w = World::make(vec![profile], true, true, false);
        w.library();
        let before = w.attachments(agent);

        let attach = w
            .svc
            .attach_preset(agent, &strings(&["R.yaml"]), None, None)
            .unwrap();
        let rev = agent_base_rev(true, &before.presets);
        let reorder = w
            .svc
            .reorder_presets(agent, &strings(&["Q.yaml", "P.yaml"]), &rev, None)
            .unwrap();
        let detach = w
            .svc
            .detach_preset(agent, &strings(&["engram:/default", "P.yaml"]), true, None)
            .unwrap();

        assert_eq!(
            w.attachments(agent),
            before,
            "the registry was written directly"
        );
        assert_eq!(
            w.port.writes(),
            vec![
                (agent, attached(true, &["P.yaml", "Q.yaml", "R.yaml"]), true),
                (agent, attached(true, &["Q.yaml", "P.yaml"]), true),
                (agent, attached(false, &["Q.yaml"]), true),
            ],
            "each write goes through the port, from inside a change"
        );
        assert_eq!(attach.applied, strings(&["R.yaml"]));
        assert_eq!(
            attach.view.presets,
            strings(&["P.yaml", "Q.yaml", "R.yaml"]),
            "the answer is the attachments as committed"
        );
        assert_eq!(reorder.presets, strings(&["Q.yaml", "P.yaml"]));
        assert_eq!(
            (detach.default_preset, detach.presets),
            (false, strings(&["Q.yaml"]))
        );
    }

    #[test]
    fn an_edit_with_a_stale_root_generation_writes_nothing() {
        let profile = claude(&[], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let current = w.svc.status().root_generation;

        let stale = w.svc.detach_preset(
            agent,
            &strings(&["engram:/default"]),
            true,
            Some(current + 1),
        );

        assert_eq!(
            code(stale),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE.to_owned())
        );
        assert!(w.port.writes().is_empty());
        assert!(w
            .svc
            .detach_preset(agent, &strings(&["engram:/default"]), true, Some(current))
            .is_ok());
    }

    // ── attach ──────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn attach_applies_what_fits_skips_the_rest_with_a_reason_and_places_after_the_anchor() {
        let profile = claude(&["P.yaml", "Q.yaml"], false);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let answer = w
            .svc
            .attach_preset(
                agent,
                &strings(&[
                    "R.yaml",
                    "engram:/default",
                    "S.yaml",
                    "Q.yaml",
                    "rules/a.md",
                    "engram:/priming",
                    "M.yaml",
                    "../x.yaml",
                    "engram:/default",
                    "R.yaml",
                ]),
                Some("P.yaml"),
                None,
            )
            .unwrap();

        let skips: Vec<(&str, &str)> = answer
            .skipped
            .iter()
            .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
            .collect();
        assert_eq!(
            answer.applied,
            strings(&["R.yaml", "engram:/default", "S.yaml"])
        );
        assert_eq!(
            skips,
            [
                ("Q.yaml", ALREADY_PRESENT),
                ("rules/a.md", WRONG_KIND),
                ("engram:/priming", WRONG_KIND),
                ("M.yaml", NOT_FOUND),
                ("../x.yaml", ITEM_PATH_INVALID),
                ("engram:/default", ALREADY_PRESENT),
                ("R.yaml", ALREADY_PRESENT),
            ]
        );
        assert_eq!(
            w.attachments(agent),
            attached(true, &["P.yaml", "R.yaml", "S.yaml", "Q.yaml"]),
            "placed after the anchor in call order"
        );
        assert_eq!(
            answer.view.presets,
            strings(&["P.yaml", "R.yaml", "S.yaml", "Q.yaml"])
        );
    }

    #[test]
    fn attach_without_an_anchor_in_the_list_appends() {
        let profile = claude(&["P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        w.svc
            .attach_preset(agent, &strings(&["Q.yaml", "R.yaml"]), Some("X.yaml"), None)
            .unwrap();

        assert_eq!(
            w.attachments(agent),
            attached(true, &["P.yaml", "Q.yaml", "R.yaml"])
        );
    }

    // ── detach ──────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn detaching_the_default_preset_needs_yes_while_the_slot_is_on() {
        let profile = claude(&["P.yaml", "Q.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let without_yes =
            w.svc
                .detach_preset(agent, &strings(&["P.yaml", "engram:/default"]), false, None);
        let (class, reason) = refused(without_yes);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, DEFAULT_PRESET_DETACH)
        );
        assert_eq!(reason.args, json!({ "agent": agent.to_string() }));
        assert!(w.port.writes().is_empty(), "the refusal applied nothing");
        assert_eq!(w.attachments(agent), attached(true, &["P.yaml", "Q.yaml"]));

        let with_yes = w
            .svc
            .detach_preset(agent, &strings(&["engram:/default"]), true, None)
            .unwrap();
        assert!(!with_yes.default_preset);
        let again = w
            .svc
            .detach_preset(agent, &strings(&["engram:/default", "P.yaml"]), false, None)
            .unwrap();
        assert!(
            !again.default_preset,
            "with the slot off there is nothing to confirm"
        );
        assert_eq!(w.attachments(agent), attached(false, &["Q.yaml"]));
        let idempotent = w
            .svc
            .detach_preset(agent, &strings(&["P.yaml"]), false, None)
            .unwrap();
        assert_eq!(idempotent.presets, strings(&["Q.yaml"]));
    }

    #[test]
    fn detach_with_a_spelling_that_is_not_a_reference_refuses_the_whole_call() {
        let profile = claude(&["P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let refusal = w
            .svc
            .detach_preset(agent, &strings(&["P.yaml", "../x.yaml"]), true, None);

        assert_eq!(
            code(refusal),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned())
        );
        assert!(w.port.writes().is_empty());
    }

    // ── baseRev and reorder ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_agent_base_rev_is_the_sha256_of_the_canonical_attachments_and_order_sensitive() {
        let ab = claude(&["A.yaml", "B.yaml"], true);
        let ba = claude(&["B.yaml", "A.yaml"], true);
        let ab_off = claude(&["A.yaml", "B.yaml"], false);
        let ids = [ab.id, ba.id, ab_off.id];
        let w = World::new(vec![ab, ba, ab_off]);
        let revs: Vec<String> = ids
            .iter()
            .map(|&id| w.svc.agent_presets(id).unwrap().base_rev)
            .collect();

        assert_eq!(
            revs[0],
            sha256_of(r#"{"default":true,"presets":["A.yaml","B.yaml"]}"#)
        );
        assert_eq!(
            revs[1],
            sha256_of(r#"{"default":true,"presets":["B.yaml","A.yaml"]}"#)
        );
        assert_eq!(
            revs[2],
            sha256_of(r#"{"default":false,"presets":["A.yaml","B.yaml"]}"#)
        );
    }

    #[test]
    fn reorder_refuses_a_base_rev_the_registry_moved_past() {
        let profile = claude(&["P.yaml", "Q.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let shown = w.svc.agent_presets(agent).unwrap().base_rev;
        // Another writer moves the attachments after the view was read.
        assert!(w.registry.update_with(agent, |p| p.default_preset = false));

        let (class, reason) =
            refused(
                w.svc
                    .reorder_presets(agent, &strings(&["Q.yaml", "P.yaml"]), &shown, None),
            );

        let current = sha256_of(r#"{"default":false,"presets":["P.yaml","Q.yaml"]}"#);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, REV_MISMATCH)
        );
        assert_eq!(reason.args, json!({ "rev": current }));
        assert!(w.port.writes().is_empty());
        let view = w
            .svc
            .reorder_presets(agent, &strings(&["Q.yaml", "P.yaml"]), &current, None)
            .unwrap();
        assert_eq!(view.presets, strings(&["Q.yaml", "P.yaml"]));
    }

    #[test]
    fn reorder_refuses_an_order_that_is_not_a_permutation() {
        let profile = claude(&["P.yaml", "Q.yaml", "P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let rev = w.svc.agent_presets(agent).unwrap().base_rev;

        for order in [
            &["P.yaml", "Q.yaml"][..],
            &["P.yaml", "Q.yaml", "Q.yaml"],
            &["P.yaml", "Q.yaml", "engram:/default"],
        ] {
            let (class, reason) =
                refused(w.svc.reorder_presets(agent, &strings(order), &rev, None));
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::InvalidArgument, REORDER_ORDER_MISMATCH),
                "{order:?}"
            );
            assert_eq!(
                reason.args,
                json!({ "expected": ["P.yaml", "Q.yaml", "P.yaml"] })
            );
        }
        assert!(w.port.writes().is_empty());
        let view = w
            .svc
            .reorder_presets(agent, &strings(&["Q.yaml", "P.yaml", "P.yaml"]), &rev, None)
            .unwrap();
        assert_eq!(view.presets, strings(&["Q.yaml", "P.yaml", "P.yaml"]));
        assert!(view.default_preset, "the Default slot is untouched");
    }

    #[test]
    fn the_base_rev_is_read_from_the_registry_inside_the_change() {
        let profile = claude(&["P.yaml", "Q.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let shown = w.svc.agent_presets(agent).unwrap().base_rev;
        // A refresh walk holds the ticket line (no library lock), so the reorder's change queues behind it after
        //   everything the reorder does before its change.
        let (arrived, at_walk) = mpsc::channel();
        let (go, released) = mpsc::channel();
        *lock(&w.store.hold) = Some((arrived, released));
        let walker = {
            let svc = Arc::clone(&w.svc);
            thread::spawn(move || drop(svc.refresh(None)))
        };
        at_walk.recv_timeout(Duration::from_secs(10)).unwrap();
        let reorder = {
            let (svc, shown) = (Arc::clone(&w.svc), shown.clone());
            thread::spawn(move || {
                svc.reorder_presets(agent, &strings(&["Q.yaml", "P.yaml"]), &shown, None)
            })
        };
        w.svc
            .gate
            .wait_for("the reorder queued behind the walk", |line, _| {
                matches!(line, [("scan", true), ("change", false)])
            });
        // Another writer moves the attachments while the reorder waits for its change.
        assert!(w.registry.update_with(agent, |p| p.default_preset = false));
        go.send(()).unwrap();
        walker.join().unwrap();

        let (class, reason) = refused(reorder.join().unwrap());
        let current = sha256_of(r#"{"default":false,"presets":["P.yaml","Q.yaml"]}"#);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, REV_MISMATCH)
        );
        assert_eq!(reason.args, json!({ "rev": current }));
        assert!(w.port.writes().is_empty());
    }

    #[test]
    fn detach_drops_an_attached_entry_that_is_not_a_reference_by_its_text() {
        let profile = claude(&["../bad.yaml", "P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let shown = w.svc.agent_presets(agent).unwrap();
        assert!(
            shown.problems.iter().any(|problem| matches!(
                problem,
                BlockReason::Broken { item, reason, .. }
                    if item == "../bad.yaml" && reason.code == ITEM_PATH_INVALID
            )),
            "{:?}",
            shown.problems
        );

        let unmatched = w.svc.detach_preset(
            agent,
            &strings(&["../bad.yaml", "../other.yaml"]),
            false,
            None,
        );
        assert_eq!(
            code(unmatched),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned()),
            "a spelling that matches no attached entry refuses the whole call"
        );
        assert!(w.port.writes().is_empty());

        let view = w
            .svc
            .detach_preset(agent, &strings(&["../bad.yaml"]), false, None)
            .unwrap();
        assert_eq!(view.presets, strings(&["P.yaml"]));
        assert_eq!(w.attachments(agent), attached(true, &["P.yaml"]));
        assert!(view.problems.is_empty(), "{:?}", view.problems);
    }

    // ── the port cell and the port's own refusals ───────────────────────────────────────────────────────────

    #[test]
    fn an_empty_or_dead_port_cell_answers_profile_edit_unavailable_and_changes_nothing() {
        let unplugged = World::make(vec![claude(&["P.yaml"], true)], true, false, true);
        let mut dead = World::new(vec![claude(&["P.yaml"], true)]);
        dead.port = Arc::new(Port::default());

        for w in [&unplugged, &dead] {
            w.library();
            let agent = w.registry.list()[0].id;
            let before = w.attachments(agent);
            let answers = [
                code(w.svc.agent_presets(agent)),
                code(
                    w.svc
                        .attach_preset(agent, &strings(&["Q.yaml"]), None, None),
                ),
                code(
                    w.svc
                        .detach_preset(agent, &strings(&["P.yaml"]), true, None),
                ),
                code(
                    w.svc
                        .reorder_presets(agent, &strings(&["P.yaml"]), "x", None),
                ),
            ];
            for answer in answers {
                assert_eq!(
                    answer,
                    (RefusalClass::Internal, PROFILE_EDIT_UNAVAILABLE.to_owned())
                );
            }
            assert_eq!(w.attachments(agent), before);
        }
    }

    #[test]
    fn a_port_store_error_is_passed_through_and_answers_as_store_command_error() {
        let profile = claude(&["P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let cases = [
            (
                StoreError::Refused(Refusal::Unreadable),
                ErrorCode::Conflict,
                RetryMode::AfterCondition,
            ),
            (
                StoreError::ReadOnly(Refusal::Newer { found: 9 }),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                StoreError::Io(std::io::Error::other("disk full")),
                ErrorCode::Internal,
                RetryMode::Never,
            ),
        ];
        for (failure, bus, retry) in cases {
            let shown = failure.to_string();
            *lock(&w.port.fail) = Some(ProfileEditError::Store(failure));

            let answer = w
                .svc
                .detach_preset(agent, &strings(&["P.yaml"]), true, None);

            let Err(LibraryRefusal::Store(store)) = answer else {
                panic!("not passed through: {answer:?}");
            };
            assert_eq!(store.to_string(), shown, "unchanged");
            let error = store_command_error(&store, String::new());
            assert_eq!((error.code(), error.retry()), (bus, retry), "{shown}");
            assert_eq!(w.attachments(agent), attached(true, &["P.yaml"]));
        }
    }

    #[test]
    fn an_agent_not_in_the_registry_answers_not_found() {
        let w = World::new(Vec::new());
        let missing = AgentId::new_v4();
        let answers = [
            code(w.svc.agent_presets(missing)),
            code(
                w.svc
                    .attach_preset(missing, &strings(&["engram:/default"]), None, None),
            ),
            code(w.svc.reorder_presets(missing, &[], "x", None)),
        ];
        for answer in answers {
            assert_eq!(answer, (RefusalClass::NotFound, NOT_FOUND.to_owned()));
        }
        // The agent removed between the registry read and the write: the port's own `NotFound`.
        let profile = claude(&[], false);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        *lock(&w.port.fail) = Some(ProfileEditError::NotFound(agent));
        assert_eq!(
            code(
                w.svc
                    .attach_preset(agent, &strings(&["engram:/default"]), None, None)
            ),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
    }

    // ── no root ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn with_no_root_only_an_attach_naming_a_library_preset_is_unavailable() {
        let profile = claude(&["P.yaml", "Q.yaml"], false);
        let agent = profile.id;
        let w = World::make(vec![profile], false, true, true);
        w.library();
        assert_eq!(w.svc.status().root, None);

        let library =
            w.svc
                .attach_preset(agent, &strings(&["engram:/default", "R.yaml"]), None, None);
        let (class, reason) = refused(library);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
        );
        assert!(w.port.writes().is_empty(), "nothing applied");

        let default = w
            .svc
            .attach_preset(agent, &strings(&["engram:/default"]), None, None)
            .unwrap();
        let rev = default.view.base_rev.clone();
        let reorder = w
            .svc
            .reorder_presets(agent, &strings(&["Q.yaml", "P.yaml"]), &rev, None)
            .unwrap();
        let detach = w
            .svc
            .detach_preset(agent, &strings(&["Q.yaml"]), false, None)
            .unwrap();

        assert_eq!(default.applied, strings(&["engram:/default"]));
        assert_eq!(reorder.presets, strings(&["Q.yaml", "P.yaml"]));
        assert_eq!(w.attachments(agent), attached(true, &["P.yaml"]));
        assert!(
            matches!(
                detach.problems.as_slice(),
                [BlockReason::LibraryUnavailable { .. }]
            ),
            "the view shows what resolution gives with no root: {:?}",
            detach.problems
        );
        assert_eq!(
            w.svc.agent_presets(agent).unwrap().problems,
            detach.problems,
            "the edit's view is the read's view"
        );
    }

    #[test]
    fn a_root_that_became_unusable_answers_the_same_block_in_the_edit_and_the_read() {
        let profile = claude(&["P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        assert!(w.svc.agent_presets(agent).unwrap().problems.is_empty());
        fs::remove_dir_all(w.base.join("lib")).unwrap();

        let detach = w
            .svc
            .detach_preset(agent, &strings(&["engram:/default"]), true, None)
            .unwrap();

        assert!(
            matches!(
                detach.problems.as_slice(),
                [BlockReason::LibraryUnavailable { .. }]
            ),
            "{:?}",
            detach.problems
        );
        assert_eq!(
            w.svc.agent_presets(agent).unwrap().problems,
            detach.problems
        );
    }

    /// With no first scan applied, the edit's answer and `agent.presets` judge the root by one rule (ADR-0384 ·
    /// ADR-0387): a root that is there is unreadable — the scan's reason, blocking; a root that is not there is absent
    /// — `missing`, skipped.
    #[test]
    fn the_edit_and_the_read_judge_a_root_whose_first_scan_failed_by_one_rule() {
        let profile = claude(&["P.yaml"], false);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.store
            .fail_scans
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let message = |problems: &[Problem]| match problems {
            [BlockReason::LibraryUnavailable { reason }] => {
                reason.args["message"].as_str().unwrap().to_owned()
            }
            other => panic!("{other:?}"),
        };

        let edit = w
            .svc
            .detach_preset(agent, &strings(&["Q.yaml"]), false, None)
            .unwrap();
        let read = w.svc.agent_presets(agent).unwrap();
        assert_eq!(
            edit.problems, read.problems,
            "the edit's view is the read's view"
        );
        let why = message(&read.problems);
        assert!(!why.is_empty() && why != "missing", "unreadable: {why}");

        fs::remove_dir_all(w.base.join("lib")).unwrap();
        let edit = w
            .svc
            .detach_preset(agent, &strings(&["Q.yaml"]), false, None)
            .unwrap();
        let read = w.svc.agent_presets(agent).unwrap();
        assert_eq!(
            edit.problems, read.problems,
            "the edit's view is the read's view"
        );
        assert_eq!(message(&read.problems), "missing", "absent");
    }

    // ── the view ────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_view_carries_the_attachments_the_lock_the_agent_values_and_the_resolution_rows() {
        let profile = claude(&["P.yaml", "M.yaml"], true);
        let agent = profile.id;
        let mut shell = claude(&["P.yaml"], true);
        shell.command = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let shell_id = shell.id;
        let w = World::new(vec![profile, shell]);
        w.library();
        *lock(&w.port.lock) = Some(WorkspaceLock::Conversed);

        let view = w.svc.agent_presets(agent).unwrap();
        let shell_view = w.svc.agent_presets(shell_id).unwrap();

        assert_eq!(
            (view.agent, view.default_preset, &view.presets, &view.cwd),
            (
                agent,
                true,
                &strings(&["P.yaml", "M.yaml"]),
                &PathBuf::from("/work")
            )
        );
        assert_eq!(view.workspace_lock, Some(WorkspaceLock::Conversed));
        assert_eq!(
            (view.backend.as_str(), view.mode),
            ("claude", AgentOutputFormat::StreamJson)
        );
        assert!(view.takes_presets);
        assert!(
            view.problems.iter().any(|problem| matches!(
                problem,
                BlockReason::Missing { item, .. } if item == "M.yaml"
            )),
            "{:?}",
            view.problems
        );
        assert!(view.conflicts.is_empty());
        assert!(
            !shell_view.takes_presets,
            "a backend with no settings check takes no preset"
        );
        assert!(shell_view.problems.is_empty());
        assert_eq!(shell_view.presets, strings(&["P.yaml"]));
    }

    // ── the `agent.new {presets}` judgement ─────────────────────────────────────────────────────────────────

    /// The judgement is `attach`'s over a profile outside the registry: the same attachments and the same skips an
    /// attach of the same presets to an identical registered agent gives — and it writes nothing (no port write, the
    /// profile not added to the registry).
    #[test]
    fn the_new_agent_judgement_is_attachs_over_a_profile_outside_the_registry() {
        let presets = strings(&[
            "R.yaml",
            "engram:/default",
            "rules/a.md",
            "engram:/priming",
            "M.yaml",
            "../x.yaml",
            "S.yaml",
            "R.yaml",
        ]);
        let new = claude(&[], true);
        let mut twin = new.clone();
        twin.id = AgentId::new_v4();
        let w = World::new(vec![twin.clone()]);
        w.library();

        let judged = w.svc.judge_new_presets(&new, &presets, None).unwrap();
        assert!(
            w.port.writes().is_empty(),
            "the judgement wrote through the port"
        );
        assert!(
            w.registry.get(new.id).is_none(),
            "the judgement added the profile"
        );
        let attach = w.svc.attach_preset(twin.id, &presets, None, None).unwrap();

        assert_eq!(judged.attachments, w.attachments(twin.id));
        assert_eq!(judged.skipped, attach.skipped);
        assert_eq!(judged.attachments, attached(true, &["R.yaml", "S.yaml"]));
    }

    /// With no root, naming a library preset refuses the whole judgement `library-unavailable` — the attach rule —
    /// while `engram:/default` alone is judged without the library.
    #[test]
    fn the_new_agent_judgement_refuses_a_library_preset_with_no_library_and_judges_the_default_alone(
    ) {
        let w = World::make(Vec::new(), false, true, true);
        let new = claude(&[], true);

        let library = w
            .svc
            .judge_new_presets(&new, &strings(&["engram:/default", "R.yaml"]), None);
        assert_eq!(
            code(library),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE.to_owned())
        );

        let default = w
            .svc
            .judge_new_presets(
                &new,
                &strings(&["engram:/default", "engram:/priming"]),
                None,
            )
            .unwrap();
        assert_eq!(default.attachments, attached(true, &[]));
        let skips: Vec<(&str, &str)> = default
            .skipped
            .iter()
            .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
            .collect();
        assert_eq!(
            skips,
            [
                ("engram:/default", ALREADY_PRESENT),
                ("engram:/priming", WRONG_KIND)
            ]
        );
    }

    /// The root generation is compared inside the judgement's read lock: a stale one refuses `root-generation-stale`
    /// with `{given, current}`, the current one is judged.
    #[test]
    fn the_new_agent_judgement_refuses_a_stale_root_generation() {
        let w = World::new(Vec::new());
        w.library();
        let new = claude(&[], true);
        let current = w.svc.status().root_generation;

        let (class, reason) = refused(w.svc.judge_new_presets(
            &new,
            &strings(&["R.yaml"]),
            Some(current + 1),
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE)
        );
        assert_eq!(
            reason.args,
            json!({ "given": current + 1, "current": current })
        );
        // Without a library preset too — the generation is compared whenever presets are judged.
        assert_eq!(
            code(
                w.svc
                    .judge_new_presets(&new, &strings(&["engram:/default"]), Some(current + 1))
            ),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE.to_owned())
        );
        let judged = w
            .svc
            .judge_new_presets(&new, &strings(&["R.yaml"]), Some(current))
            .unwrap();
        assert_eq!(judged.attachments, attached(true, &["R.yaml"]));
    }

    /// With no library service, a library preset refuses the whole judgement `library-unavailable` with the given
    /// reason, and the rest is judged as the service judges it.
    #[test]
    fn the_judgement_with_no_library_service_refuses_a_library_preset_and_judges_the_rest() {
        let new = claude(&[], false);

        let (class, reason) = refused(judge_new_presets_without_library(
            &new,
            &strings(&["engram:/default", "R.yaml"]),
            "no service",
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
        );
        assert_eq!(reason.args, json!({ "message": "no service" }));

        let judged = judge_new_presets_without_library(
            &new,
            &strings(&["engram:/default", "../x.yaml"]),
            "no service",
        )
        .unwrap();
        assert_eq!(judged.attachments, attached(true, &[]));
        let skips: Vec<(&str, &str)> = judged
            .skipped
            .iter()
            .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
            .collect();
        assert_eq!(skips, [("../x.yaml", ITEM_PATH_INVALID)]);
    }
}
