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
//! - **An agent takes presets only**: a library preset, or `engram/default` for its Default slot.
//! - **Only an `attach` naming a library preset needs the library.** It refuses `Unavailable` · `library-unavailable`
//!   when no first scan could be applied or the mounts are unknown (`library.json` unusable), and — with `mount` —
//!   when a named preset's mount cannot be read (never walked this run, or not `Ok` when last seen, after a walk
//!   again; ADR-0400 decision 19), nothing applied. `detach`, `reorder` and an `attach` of `engram/default` alone
//!   judge nothing against the catalog and work with no mount; their answer view shows the rows resolution gives
//!   under those mount states.
//! - **Answers are built inside the change.** The answer view is resolved inside the change from the attachments as
//!   committed; its folder stats and link probes run after it, outside every lock. A mount the committed attachments
//!   reach that the edit did not stat (another writer attached them in between) is stat'ed then too, and the committed
//!   attachments are resolved again under one read lock, so the view still equals `agent.presets` (ADR-0384).
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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use engram_dashboard_agent::launch::{backend_word, BlockReason, Notice, Reason};
use engram_dashboard_agent::profile::{AgentOutputFormat, AgentProfile, AttachedPreset};
use engram_dashboard_agent::profile_edit::{
    Attachments, ProfileEditError, ProfileEditPort, WorkspaceLock,
};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_platform::fs::{path_key, PathKey};
use serde_json::json;

use crate::catalog::Catalog;
use crate::config::ConfigHealth;
use crate::edit::{
    list_of, parse_ref, skip, Skipped, ALREADY_PRESENT, REORDER_ORDER_MISMATCH, REV_MISMATCH,
};
use crate::folder::FolderStatus;
use crate::format::read::WRONG_KIND;
use crate::format::ListName;
use crate::gate::NOT_FOUND;
use crate::launch::sha256_hex;
use crate::mounts::{Mount, MountName, Mounts};
use crate::path::{AssetRef, BuiltinId};
use crate::resolve::{
    ref_key, resolve, CatalogLookup, Looked, Lookup, MountStates, Problem, RefKey, Resolution,
    ResolvedFolder, TagRule, Target,
};
use crate::{
    output_format, reached_mounts, read_states, refuse_unusable_now, stale_root_generation,
    LibraryRefusal, LibraryService, MountSeen, MountsNow, RefusalClass,
};

/// The code of a `detach` of `engram/default` without `yes` while the agent's Default slot is on (`args` =
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
    /// The Default slot (`engram/default`).
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
    /// library row of an absent mount) and refuses on the others.
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
    /// The mounts its link probes run in.
    mounts: Mounts,
    /// The mounts the committed attachments reach that the edit did not stat before its change — another writer
    /// attached them in between; the resolution judged them by their last known state until they are stat'ed.
    unprobed: Vec<Mount>,
}

/// The first-scan entrance's answer and the mount stats an edit took before its change, outside every lock.
struct RootSeen {
    joined: Result<(), String>,
    /// The folder of each mount the agent's attachments or the request reach, as stat'ed and judged with its first-scan
    /// answer (`LibraryService::stat_reached`), by path key — taken the first scan failed or not.
    seen: HashMap<PathKey, FolderStatus>,
}

impl RootSeen {
    /// The mount states the answer view resolves under, for `mounts` as the change or read lock found them and their
    /// last known states `seen` — the rule of the read path ([`crate::read_states`]), so the edit's answer equals
    /// `agent.presets` (ADR-0384).
    // ADR-0387
    // ADR-0400 decision 19
    fn states(&self, mounts: &Mounts, seen: impl Fn(&MountName) -> MountSeen) -> MountStates {
        read_states(mounts, seen, &self.seen, &self.joined)
    }

    /// The mounts `profile`'s library attachments reach in `catalog` by its tag rule `rule` that were not stat'ed before
    /// the change.
    fn unprobed(
        &self,
        catalog: &Catalog,
        mounts: &Mounts,
        profile: &AgentProfile,
        rule: TagRule,
    ) -> Vec<Mount> {
        let refs: Vec<AssetRef> = profile
            .presets
            .iter()
            .filter_map(|preset| AssetRef::parse(&preset.path).ok())
            .collect();
        let refs: Vec<(TagRule, &AssetRef)> = refs.iter().map(|r| (rule, r)).collect();
        reached_mounts(catalog, mounts, &refs)
            .into_iter()
            .filter(|mount| !self.seen.contains_key(&path_key(&mount.folder)))
            .collect()
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
    /// anchor in call order (no anchor in the list = at its end); `engram/default` turns the Default slot on. Each one
    /// that cannot be attached is skipped with a reason: not a reference (`item-path-invalid`), not in the catalog
    /// (`not-found`), not a preset (`wrong-kind`), already attached ([`ALREADY_PRESENT`], by catalog key). Naming a
    /// library preset with no first scan applied, or while the mounts are unknown (`library.json` unusable), refuses
    /// `Unavailable` · `library-unavailable`, nothing applied; so does one whose mount cannot be read, with `mount`
    /// (ADR-0400 decision 19). Plus the refusals every edit can answer (module header).
    pub fn attach_preset(
        &self,
        agent: AgentId,
        presets: &[String],
        after: Option<&str>,
        root_generation: Option<u64>,
    ) -> Result<AgentAttach, LibraryRefusal> {
        let library = names_library(presets);
        let ((applied, skipped), view) = self.agent_edit(
            agent,
            root_generation,
            library,
            presets,
            |profile, lookup| Ok(attach(profile, lookup, presets, after)),
        )?;
        Ok(AgentAttach {
            applied,
            skipped,
            view,
        })
    }

    /// Detaches `presets` from `agent`: every attached entry with the same catalog key goes; `engram/default` turns
    /// the Default slot off. A spelling that is not a reference matches an attached entry by its exact text, so a
    /// stored entry that does not parse can be dropped. Idempotent for references (a repeated raw spelling refuses once its
    /// entry is gone) and not edition-checked. A spelling that is not a
    /// reference and matches no attached entry refuses the whole call `InvalidArgument` · `item-path-invalid`;
    /// `engram/default` while the slot is on and `yes` is not given refuses `Conflict` · [`DEFAULT_PRESET_DETACH`].
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
        let ((), view) = self.agent_edit(agent, root_generation, false, &[], |profile, _| {
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
        let ((), view) = self.agent_edit(agent, root_generation, false, &[], |profile, _| {
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
    /// one read lock. Nothing is written and no port is needed. Naming a library preset with no first scan applied, or
    /// while the mounts are unknown (`library.json` unusable), refuses `Unavailable` · `library-unavailable`, as does
    /// naming one whose mount cannot be read, with `mount` (ADR-0400 decision 19); a stale `root_generation`, compared
    /// inside the read lock, refuses `Conflict` · `root-generation-stale`.
    // ADR-0386
    pub fn judge_new_presets(
        &self,
        profile: &AgentProfile,
        presets: &[String],
        root_generation: Option<u64>,
    ) -> Result<NewPresets, LibraryRefusal> {
        // Only a library preset is looked up in the catalog, so only then is the first scan waited for.
        if names_library(presets) {
            self.refuse_unknown_mounts()?;
            self.first_scan_join()
                .map_err(LibraryRefusal::unavailable)?;
            self.refuse_unreadable_named(presets)?;
        }
        self.gate.with_read(|state| {
            let current = self.gate.numbers().root;
            if root_generation.is_some_and(|given| given != current) {
                return Err(stale_root_generation(root_generation, current));
            }
            refuse_unreadable_named_now(&state.mounts_now(), presets)?;
            let lookup = CatalogLookup {
                catalog: &state.catalog,
                mounts: &state.mounts,
                builtins: &self.builtins,
            };
            let (attachments, (_, skipped)) = attach(profile, &lookup, presets, None);
            Ok(NewPresets {
                attachments,
                skipped,
            })
        })
    }

    /// While `library.json` could not be read (corrupt, newer or unreadable) the run does not know its mounts, so a
    /// named library preset cannot be judged: `Unavailable` · `library-unavailable`, nothing applied. A valid file with
    /// no mount knows its mounts (none), and its presets are judged — and skipped as `not-found`.
    // ADR-0386 decision 5
    // ADR-0400
    fn refuse_unknown_mounts(&self) -> Result<(), LibraryRefusal> {
        let (mounts, health) = self.config.view();
        if mounts.is_some() {
            return Ok(());
        }
        let why = match health {
            ConfigHealth::ReadOnly { why } => {
                format!("library.json cannot be used this run: {why}")
            }
            _ => "library.json is corrupt".to_owned(),
        };
        Err(LibraryRefusal::unavailable(format!(
            "the library mounts are unknown: {why}"
        )))
    }

    /// The attach rule per mount (ADR-0386 decision 5 · ADR-0400 decision 19): each library preset `presets` name whose
    /// mount cannot be read after a walk again refuses the whole call `Unavailable` · `library-unavailable` `{mount,
    /// message}` ([`LibraryService::refuse_unusable_mount`]). A preset in no registered mount passes (it is skipped as
    /// `not-found`).
    fn refuse_unreadable_named(&self, presets: &[String]) -> Result<(), LibraryRefusal> {
        for path in library_paths(presets) {
            self.refuse_unusable_mount(&path)?;
        }
        Ok(())
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
    /// edit judges library presets against the catalog), the stats of the mounts the answer view can name — those the
    /// agent's attachments and the request's `named` spellings name, as `agent.presets` stats the mounts its
    /// attachments name (ADR-0384) — then one [`LibraryService::change_at`] ([`LibraryService::agent_edit_in`]); the
    /// answer view is settled after it.
    fn agent_edit<T>(
        &self,
        agent: AgentId,
        root_generation: Option<u64>,
        library: bool,
        named: &[String],
        op: impl FnOnce(&AgentProfile, &CatalogLookup<'_>) -> Result<(Attachments, T), LibraryRefusal>,
    ) -> Result<(T, AgentPresetsView), LibraryRefusal> {
        let port = self.profile_edit()?;
        if library {
            self.refuse_unknown_mounts()?;
        }
        let joined = self.first_scan_join();
        if let (true, Err(why)) = (library, &joined) {
            return Err(LibraryRefusal::unavailable(why.clone()));
        }
        if library {
            self.refuse_unreadable_named(named)?;
        }
        // The stats run here, outside every lock, in one batch.
        let profile = self.values.registry.get(agent);
        let rule = match &profile {
            Some(profile) => self.tag_rule(profile),
            None => TagRule::of(&*self.values.backends, None, None),
        };
        let refs: Vec<AssetRef> = profile
            .iter()
            .flat_map(|profile| &profile.presets)
            .map(|preset| preset.path.as_str())
            .chain(named.iter().map(String::as_str))
            .filter_map(|text| AssetRef::parse(text).ok())
            .collect();
        let refs: Vec<(TagRule, &AssetRef)> = refs.iter().map(|r| (rule, r)).collect();
        let reached = self
            .gate
            .with_read(|state| reached_mounts(&state.catalog, &state.mounts, &refs));
        let seen = self.stat_reached(&self.io.begin_read(), &reached);
        let before = RootSeen { joined, seen };
        let (answer, pending) = self.change_at(root_generation, |catalog, mounts, _| {
            // ADR-0400 decision 6: a scan queued ahead of the change may have found a named mount gone since.
            let edited = match library {
                true => refuse_unreadable_named_now(mounts, named),
                false => Ok(()),
            }
            .and_then(|()| self.agent_edit_in(&*port, agent, catalog, mounts, &before, op));
            (edited, Vec::new())
        })??;
        Ok((answer, self.settle_agent(pending, before)))
    }

    /// The steps of [`LibraryService::agent_edit`] inside the change: the registry read, `op` over the profile and the
    /// catalog, the port's lock read and write, and the answer view of the attachments as committed.
    fn agent_edit_in<T>(
        &self,
        port: &dyn ProfileEditPort,
        agent: AgentId,
        catalog: &Catalog,
        mounts: &MountsNow,
        before: &RootSeen,
        op: impl FnOnce(&AgentProfile, &CatalogLookup<'_>) -> Result<(Attachments, T), LibraryRefusal>,
    ) -> Result<(T, PendingAgentView), LibraryRefusal> {
        let lookup = CatalogLookup {
            catalog,
            mounts,
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
        let unprobed = before.unprobed(catalog, mounts, &committed, self.tag_rule(&committed));
        let states = before.states(mounts, |name| mounts.seen(name));
        let resolution = self.resolve_profile(&committed, &lookup, states);
        let pending = PendingAgentView {
            profile: committed,
            workspace_lock,
            resolution,
            mounts: Mounts::clone(mounts),
            unprobed,
        };
        Ok((answer, pending))
    }

    /// Finishes an edit's answer view outside every lock. The mounts the committed attachments reach that the edit did
    /// not stat before its change (another writer attached them in between) are stat'ed now, in one batch, and the
    /// committed attachments are resolved again under one read lock with those stats, as `agent.presets` would resolve
    /// them. Nothing new to stat = no extra IO; the edit itself is never retried.
    // ADR-0384
    // ADR-0387
    fn settle_agent(&self, pending: PendingAgentView, before: RootSeen) -> AgentPresetsView {
        let PendingAgentView {
            profile,
            workspace_lock,
            mut resolution,
            mut mounts,
            unprobed,
        } = pending;
        if !unprobed.is_empty() {
            let mut seen = before.seen;
            seen.extend(self.stat_reached(&self.io.begin_read(), &unprobed));
            let seen = RootSeen {
                joined: before.joined,
                seen,
            };
            (resolution, mounts) = self.resolve_committed(&profile, &seen);
        }
        if let Some(resolution) = &mut resolution {
            self.settle_resolution(resolution, &mounts);
        }
        view_of(&profile, workspace_lock, resolution)
    }

    /// `profile`'s attachments resolved under one read lock with the mount stats of `seen`, and the mounts it read.
    fn resolve_committed(
        &self,
        profile: &AgentProfile,
        seen: &RootSeen,
    ) -> (Option<Resolution>, Mounts) {
        self.gate.with_read(|state| {
            let lookup = CatalogLookup {
                catalog: &state.catalog,
                mounts: &state.mounts,
                builtins: &self.builtins,
            };
            let states = seen.states(&state.mounts, |name| state.seen(name));
            (
                self.resolve_profile(profile, &lookup, states),
                state.mounts.clone(),
            )
        })
    }

    /// `profile`'s attachments resolved over `lookup` under the mount states `states` (`None` = it takes no preset) —
    /// the answer view's resolution.
    fn resolve_profile(
        &self,
        profile: &AgentProfile,
        lookup: &CatalogLookup<'_>,
        states: MountStates,
    ) -> Option<Resolution> {
        self.want_of(profile).map(|want| {
            let target = Target {
                backend: want.backend,
                mode: want.mode,
                default_preset: want.default_preset,
                presets: want.presets,
                unparsed: want.unparsed,
                mounts: states,
            };
            resolve(&target, lookup, &*self.values.backends)
        })
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

/// The library presets `presets` name, as parsed.
fn library_paths(presets: &[String]) -> impl Iterator<Item = crate::path::LibPath> + '_ {
    presets
        .iter()
        .filter_map(|preset| match AssetRef::parse(preset) {
            Ok(AssetRef::Library(path)) => Some(path),
            _ => None,
        })
}

/// [`LibraryService::refuse_unreadable_named`] under a library lock, from the recorded mount states only — no IO, no
/// walk again ([`refuse_unusable_now`]).
// ADR-0400 decision 19
fn refuse_unreadable_named_now(
    mounts: &MountsNow,
    presets: &[String],
) -> Result<(), LibraryRefusal> {
    for path in library_paths(presets) {
        refuse_unusable_now(mounts, &path)?;
    }
    Ok(())
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

    use std::path::Path;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, MountPlace, MountSetting};
    use crate::folder::{
        BoundedIo, FolderListing, FolderOps, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT,
        LIST_CAP, TIMEOUT,
    };
    use crate::format::read::ITEM_PATH_INVALID;
    use crate::mounts::MountName;
    use crate::mounts::MountPath;
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
            under: Option<&MountPath>,
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
        fn first_link(&self, root: &Path, rel: &MountPath) -> std::io::Result<Option<MountPath>> {
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
            let config = move |base: &Path, state: &Path| match root {
                true => LibraryConfig::usable(state, MountPlace::Explicit(base.join("lib"))),
                false => LibraryConfig::missing(state),
            };
            World::make_with(
                profiles,
                config,
                plugged,
                applies,
                Arc::new(StdFolderOps),
                IO_LIMIT,
            )
        }

        /// [`World::make`] with the config built from the temp folder and the state folder, the folder ops and the
        /// read budget given.
        fn make_with(
            profiles: Vec<AgentProfile>,
            config: impl FnOnce(&Path, &Path) -> LibraryConfig,
            plugged: bool,
            applies: bool,
            ops: Arc<dyn FolderOps>,
            limit: Duration,
        ) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            let state = base.join("state");
            fs::create_dir_all(&state).unwrap();
            let registry = Arc::new(ProfileRegistry::new(Arc::new(Saves(profiles))));
            let config = config(&base, &state);
            let values = LibraryValues {
                registry: Arc::clone(&registry),
                backends: Arc::new(AgentBackends),
                config,
                default_root: base.join("default-library"),
                data_root: base.join("data"),
                daemon_id: "test-daemon".to_owned(),
                root_generation_base: 7,
                builtin_dir: None,
                launch_root: base.join("launch"),
                home: None,
                env: Arc::new(|_| None),
            };
            let io = BoundedIo::new(
                limit,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                ops,
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

    /// [`StdFolderOps`] whose stat of one folder answers only after `delay` — a mount that does not answer in time.
    struct SlowStat {
        slow: PathBuf,
        delay: Duration,
    }

    impl FolderOps for SlowStat {
        fn stat(&self, p: &Path) -> FolderStatus {
            if p == self.slow {
                thread::sleep(self.delay);
            }
            StdFolderOps.stat(p)
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> std::io::Result<PathBuf> {
            StdFolderOps.canonical(p)
        }
        fn identity(
            &self,
            p: &Path,
        ) -> std::io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> std::io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

    /// Mounts `team` at `<base>\team` (slow to stat: past the 300 ms read budget) and then `library` at `<base>\lib`.
    fn slow_team_world(profiles: Vec<AgentProfile>) -> World {
        let slow = Arc::new(std::sync::OnceLock::<PathBuf>::new());
        let config = {
            let slow = Arc::clone(&slow);
            move |base: &Path, state: &Path| {
                let team = base.join("team");
                fs::create_dir_all(&team).unwrap();
                let _ = slow.set(team.clone());
                let mount = |name: &str, folder: PathBuf| MountSetting {
                    name: MountName::parse(name).unwrap(),
                    place: MountPlace::Explicit(folder),
                };
                LibraryConfig::mounted(
                    state,
                    vec![mount("team", team), mount("library", base.join("lib"))],
                )
            }
        };
        let ops = Arc::new(LateSlow(Arc::clone(&slow)));
        World::make_with(
            profiles,
            config,
            true,
            true,
            ops,
            Duration::from_millis(300),
        )
    }

    /// [`SlowStat`] over a folder named after the world is made.
    struct LateSlow(Arc<std::sync::OnceLock<PathBuf>>);

    impl FolderOps for LateSlow {
        fn stat(&self, p: &Path) -> FolderStatus {
            let slow = SlowStat {
                slow: self.0.get().cloned().unwrap_or_default(),
                delay: Duration::from_secs(2),
            };
            slow.stat(p)
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> std::io::Result<PathBuf> {
            StdFolderOps.canonical(p)
        }
        fn identity(
            &self,
            p: &Path,
        ) -> std::io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> std::io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

    // ── mount stats ─────────────────────────────────────────────────────────────────────────────────────────

    // ADR-0360: one batch, one wait — a mount that does not answer leaves the others their answers.
    #[test]
    fn mount_stats_are_one_batch_so_a_mount_that_does_not_answer_does_not_degrade_another() {
        let w = slow_team_world(Vec::new());
        let (team, lib) = (w.base.join("team"), w.base.join("lib"));
        let began = std::time::Instant::now();
        let statuses = w.svc.stat_mounts(&w.svc.io.begin_read(), &[team, lib]);
        assert_eq!(
            statuses,
            [
                FolderStatus::Unreadable(TIMEOUT.to_owned()),
                FolderStatus::Ok
            ],
            "the healthy mount after the slow one is still answered"
        );
        assert!(
            began.elapsed() < Duration::from_millis(1_500),
            "one wait of the budget: {:?}",
            began.elapsed()
        );
    }

    // ADR-0384: an agent-preset edit stats only the mounts its view names, as `agent.presets` does, so both answer the
    //   same and an unrelated mount that does not answer neither blocks the view nor delays the edit.
    #[test]
    fn an_agent_edit_stats_only_the_mounts_its_view_names_and_answers_as_agent_presets() {
        let profile = claude(&["library/P.yaml"], false);
        let agent = profile.id;
        let w = slow_team_world(vec![profile]);
        w.library();
        let began = std::time::Instant::now();
        let edited = w
            .svc
            .attach_preset(agent, &strings(&["library/Q.yaml"]), None, None)
            .unwrap();
        assert!(
            began.elapsed() < Duration::from_millis(1_500),
            "the slow mount was not stat'ed: {:?}",
            began.elapsed()
        );
        assert_eq!(edited.applied, ["library/Q.yaml"]);
        let view = w.svc.agent_presets(agent).unwrap();
        assert_eq!(
            edited.view, view,
            "the edit answers what agent.presets answers"
        );
        assert!(
            !view
                .problems
                .iter()
                .any(|problem| matches!(problem, BlockReason::LibraryUnavailable { .. })),
            "{:?}",
            view.problems
        );
    }

    /// [`StdFolderOps`] whose stat of `team`'s folder answers unreadable and whose stat of `lib`'s folder first runs a
    /// hook — another writer acting between an edit's probe and its change. The folders are set when the world is made.
    #[derive(Default)]
    struct Hooked {
        lib: std::sync::OnceLock<PathBuf>,
        team: std::sync::OnceLock<PathBuf>,
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl FolderOps for Hooked {
        fn stat(&self, p: &Path) -> FolderStatus {
            if self.team.get().is_some_and(|team| team == p) {
                return FolderStatus::Unreadable("access denied".to_owned());
            }
            if self.lib.get().is_some_and(|lib| lib == p) {
                if let Some(hook) = lock(&self.hook).take() {
                    hook();
                }
            }
            StdFolderOps.stat(p)
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> std::io::Result<PathBuf> {
            StdFolderOps.canonical(p)
        }
        fn identity(
            &self,
            p: &Path,
        ) -> std::io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> std::io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

    // ADR-0384 · ADR-0387: a mount another writer attaches between an edit's probe and its change is stat'ed after the
    //   change, so the edit's answer shows it unreadable exactly as `agent.presets` does.
    #[test]
    fn a_mount_attached_between_an_edits_probe_and_its_change_is_stat_ed_for_the_answer_view() {
        let profile = claude(&["library/P.yaml"], false);
        let agent = profile.id;
        let ops = Arc::new(Hooked::default());
        let config = {
            let ops = Arc::clone(&ops);
            move |base: &Path, state: &Path| {
                let (lib, team) = (base.join("lib"), base.join("team"));
                fs::create_dir_all(&team).unwrap();
                fs::write(team.join("T.yaml"), "kind: preset\nversion: 1\n").unwrap();
                let _ = ops.lib.set(lib.clone());
                let _ = ops.team.set(team.clone());
                let mount = |name: &str, folder: PathBuf| MountSetting {
                    name: MountName::parse(name).unwrap(),
                    place: MountPlace::Explicit(folder),
                };
                LibraryConfig::mounted(state, vec![mount("library", lib), mount("team", team)])
            }
        };
        let w = World::make_with(
            vec![profile],
            config,
            true,
            true,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            IO_LIMIT,
        );
        w.library();
        w.svc.first_scan_join().unwrap();
        let registry = Arc::clone(&w.registry);
        // Edit B: `team/T.yaml` attached after A's probe, before A's change.
        *lock(&ops.hook) = Some(Box::new(move || {
            registry
                .try_update_with(agent, |profile| {
                    profile.presets.push(AttachedPreset {
                        path: "team/T.yaml".to_owned(),
                    });
                })
                .unwrap();
        }));
        let edited = w
            .svc
            .attach_preset(agent, &strings(&["library/Q.yaml"]), None, None)
            .unwrap();
        assert!(
            lock(&ops.hook).is_none(),
            "B landed between A's probe and its change"
        );
        assert_eq!(
            edited.view.presets,
            ["library/P.yaml", "team/T.yaml", "library/Q.yaml"]
        );
        let unavailable = |view: &AgentPresetsView| {
            view.problems
                .iter()
                .any(|problem| matches!(problem, BlockReason::LibraryUnavailable { .. }))
        };
        assert!(unavailable(&edited.view), "{:?}", edited.view.problems);
        assert_eq!(
            edited.view,
            w.svc.agent_presets(agent).unwrap(),
            "the edit answers what agent.presets answers"
        );
    }

    /// [`StdFolderOps`] for a world over `lib` and `team`: `lib` answers unreadable while `lib_down`; the first stat of
    /// `lib` once `armed` records `team` gone (`Gate::observe`, no ticket — what a scan queued ahead of a change would
    /// find), and from then on `team` stats missing. The folders and the service are set when the world is made.
    #[derive(Default)]
    struct Observing {
        lib: std::sync::OnceLock<PathBuf>,
        team: std::sync::OnceLock<PathBuf>,
        svc: std::sync::OnceLock<Weak<LibraryService>>,
        lib_down: std::sync::atomic::AtomicBool,
        armed: std::sync::atomic::AtomicBool,
        team_gone: std::sync::atomic::AtomicBool,
    }

    impl FolderOps for Observing {
        fn stat(&self, p: &Path) -> FolderStatus {
            use std::sync::atomic::Ordering::SeqCst;
            if self.team.get().is_some_and(|team| team == p) && self.team_gone.load(SeqCst) {
                return FolderStatus::Missing;
            }
            if self.lib.get().is_some_and(|lib| lib == p) {
                if self.lib_down.load(SeqCst) {
                    return FolderStatus::Unreadable("down".to_owned());
                }
                if self.armed.swap(false, SeqCst) {
                    let svc = self.svc.get().and_then(Weak::upgrade).expect("the service");
                    let team = svc.with_read(|state| state.mounts.get("team").cloned().unwrap());
                    svc.gate.observe(vec![team], &|mounts| {
                        vec![FolderStatus::Missing; mounts.len()]
                    });
                    self.team_gone.store(true, SeqCst);
                }
            }
            StdFolderOps.stat(p)
        }
        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            StdFolderOps.list(p, cap)
        }
        fn canonical(&self, p: &Path) -> std::io::Result<PathBuf> {
            StdFolderOps.canonical(p)
        }
        fn identity(
            &self,
            p: &Path,
        ) -> std::io::Result<engram_dashboard_platform::fs::FileIdentity> {
            StdFolderOps.identity(p)
        }
        fn read(&self, p: &Path, cap: u64) -> std::io::Result<String> {
            StdFolderOps.read(p, cap)
        }
    }

    // ADR-0400 decision 19 (ADR-0386 decision 5 per mount): `agent.attachPreset` and `agent.new` judge each named
    //   preset's mount again inside the lock that applies them — a mount found gone after the entrance passed it (here
    //   while the entrance walked another named mount again) refuses the call with `mount`, nothing applied.
    #[test]
    fn attach_and_new_refuse_a_named_mount_found_gone_after_the_entrance_passed_it() {
        use std::sync::atomic::Ordering::SeqCst;
        for new in [false, true] {
            let profile = claude(&[], false);
            let agent = profile.id;
            let ops = Arc::new(Observing::default());
            ops.lib_down.store(true, SeqCst);
            let config = {
                let ops = Arc::clone(&ops);
                move |base: &Path, state: &Path| {
                    let (lib, team) = (base.join("lib"), base.join("team"));
                    fs::create_dir_all(&team).unwrap();
                    fs::write(lib.join("P.yaml"), "kind: preset\nversion: 1\n").unwrap();
                    fs::write(team.join("T.yaml"), "kind: preset\nversion: 1\n").unwrap();
                    let _ = ops.lib.set(lib.clone());
                    let _ = ops.team.set(team.clone());
                    let mount = |name: &str, folder: PathBuf| MountSetting {
                        name: MountName::parse(name).unwrap(),
                        place: MountPlace::Explicit(folder),
                    };
                    LibraryConfig::mounted(state, vec![mount("library", lib), mount("team", team)])
                }
            };
            let w = World::make_with(
                vec![profile.clone()],
                config,
                true,
                true,
                Arc::clone(&ops) as Arc<dyn FolderOps>,
                IO_LIMIT,
            );
            let _ = ops.svc.set(Arc::downgrade(&w.svc));
            // The first scan walks `team` only; `library` waits for its next reader.
            w.svc.first_scan_join().unwrap();
            ops.lib_down.store(false, SeqCst);
            ops.armed.store(true, SeqCst);
            let named = strings(&["team/T.yaml", "library/P.yaml"]);
            let (class, reason) = match new {
                false => refused(w.svc.attach_preset(agent, &named, None, None)),
                true => refused(w.svc.judge_new_presets(&profile, &named, None)),
            };
            assert!(
                ops.team_gone.load(SeqCst),
                "new: {new}: team was found gone mid-call"
            );
            assert_eq!(
                (class, reason.code.as_str(), &reason.args["mount"]),
                (
                    RefusalClass::Unavailable,
                    LIBRARY_UNAVAILABLE,
                    &json!("team")
                ),
                "new: {new}"
            );
            assert_eq!(
                w.attachments(agent),
                attached(false, &[]),
                "new: {new}: nothing applied"
            );
        }
    }

    // ── an unknown mount list ───────────────────────────────────────────────────────────────────────────────

    /// A world whose `library.json` holds `text`, read as a daemon start reads it.
    fn config_world(profiles: Vec<AgentProfile>, text: &'static str) -> World {
        let config = move |_: &Path, state: &Path| {
            fs::write(state.join(crate::config::LIBRARY_CONFIG_FILE), text).unwrap();
            crate::config::load(state)
        };
        World::make_with(
            profiles,
            config,
            true,
            true,
            Arc::new(StdFolderOps),
            IO_LIMIT,
        )
    }

    // ADR-0386 decision 5 · ADR-0400: with `library.json` unusable the run does not know its mounts, so a named library
    //   preset refuses the whole call; a valid file naming no mount judges it as not found.
    #[test]
    fn attach_and_new_refuse_library_unavailable_while_the_mounts_are_unknown_and_skip_with_an_empty_list(
    ) {
        let new_profile = claude(&[], false);
        for text in ["not json", r#"{"schema_version": 9, "mounts": []}"#] {
            let profile = claude(&[], false);
            let agent = profile.id;
            let w = config_world(vec![profile], text);
            let (class, reason) =
                refused(
                    w.svc
                        .attach_preset(agent, &strings(&["library/P.yaml"]), None, None),
                );
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE),
                "{text}"
            );
            assert!(
                reason.args["message"]
                    .as_str()
                    .unwrap()
                    .contains("mounts are unknown"),
                "{reason:?}"
            );
            assert_eq!(
                w.attachments(agent),
                attached(false, &[]),
                "{text}: nothing applied"
            );
            assert_eq!(
                code(
                    w.svc
                        .judge_new_presets(&new_profile, &strings(&["library/P.yaml"]), None)
                ),
                (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE.to_owned()),
                "{text}"
            );
            let builtin = w
                .svc
                .judge_new_presets(&new_profile, &strings(&["engram/default"]), None)
                .unwrap();
            assert!(
                builtin.attachments.default_preset,
                "{text}: a built-in alone needs no library"
            );
        }

        let profile = claude(&[], false);
        let agent = profile.id;
        let w = config_world(vec![profile], r#"{"schema_version": 2, "mounts": []}"#);
        let attach = w
            .svc
            .attach_preset(agent, &strings(&["library/P.yaml"]), None, None)
            .unwrap();
        assert!(attach.applied.is_empty());
        assert_eq!(attach.skipped.len(), 1);
        assert_eq!(
            attach.skipped[0].reason.code, NOT_FOUND,
            "{:?}",
            attach.skipped
        );
        let judged = w
            .svc
            .judge_new_presets(&new_profile, &strings(&["library/P.yaml"]), None)
            .unwrap();
        assert_eq!(judged.skipped.len(), 1);
        assert_eq!(
            judged.skipped[0].reason.code, NOT_FOUND,
            "{:?}",
            judged.skipped
        );
    }

    // ── writes through the port ─────────────────────────────────────────────────────────────────────────────

    // ADR-0400 decision 19 (ADR-0386 decision 5 per mount): naming a library preset whose mount cannot be read — here
    //   never walked, its folder missing at the first scan — refuses the whole call with `mount`, nothing applied; a
    //   preset of a readable mount alone attaches; once the mount is back, the call that names it walks it again.
    #[test]
    fn attach_and_new_refuse_a_preset_whose_mount_cannot_be_read_naming_the_mount() {
        let profile = claude(&[], false);
        let agent = profile.id;
        let w = World::make_with(
            vec![profile.clone()],
            |base: &Path, state: &Path| {
                let mount = |name: &str, folder: PathBuf| MountSetting {
                    name: MountName::parse(name).unwrap(),
                    place: MountPlace::Explicit(folder),
                };
                LibraryConfig::mounted(
                    state,
                    vec![
                        mount("library", base.join("lib")),
                        mount("team", base.join("team")),
                    ],
                )
            },
            true,
            true,
            Arc::new(StdFolderOps),
            IO_LIMIT,
        );
        w.library();
        let named = strings(&["library/P.yaml", "team/T.yaml"]);
        let (class, reason) = refused(w.svc.attach_preset(agent, &named, None, None));
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (
                RefusalClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                &json!("team")
            )
        );
        assert_eq!(
            w.attachments(agent),
            attached(false, &[]),
            "nothing applied"
        );
        let (class, reason) = refused(w.svc.judge_new_presets(&profile, &named, None));
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (
                RefusalClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                &json!("team")
            )
        );

        let alone = w
            .svc
            .attach_preset(agent, &strings(&["library/P.yaml"]), None, None)
            .unwrap();
        assert_eq!(alone.applied, ["library/P.yaml"]);

        let team = w.base.join("team");
        fs::create_dir_all(&team).unwrap();
        fs::write(team.join("T.yaml"), "kind: preset\nversion: 1\n").unwrap();
        let back = w
            .svc
            .attach_preset(agent, &strings(&["team/T.yaml"]), None, None)
            .unwrap();
        assert_eq!(back.applied, ["team/T.yaml"]);
        assert!(back.view.problems.is_empty(), "{:?}", back.view.problems);
    }

    #[test]
    fn attach_detach_and_reorder_write_only_through_the_port_from_inside_a_change() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], true);
        let agent = profile.id;
        // The port logs and does not write: whatever reaches the registry did not come through it.
        let w = World::make(vec![profile], true, true, false);
        w.library();
        let before = w.attachments(agent);

        let attach = w
            .svc
            .attach_preset(agent, &strings(&["library/R.yaml"]), None, None)
            .unwrap();
        let rev = agent_base_rev(true, &before.presets);
        let reorder = w
            .svc
            .reorder_presets(
                agent,
                &strings(&["library/Q.yaml", "library/P.yaml"]),
                &rev,
                None,
            )
            .unwrap();
        let detach = w
            .svc
            .detach_preset(
                agent,
                &strings(&["engram/default", "library/P.yaml"]),
                true,
                None,
            )
            .unwrap();

        assert_eq!(
            w.attachments(agent),
            before,
            "the registry was written directly"
        );
        assert_eq!(
            w.port.writes(),
            vec![
                (
                    agent,
                    attached(
                        true,
                        &["library/P.yaml", "library/Q.yaml", "library/R.yaml"]
                    ),
                    true
                ),
                (
                    agent,
                    attached(true, &["library/Q.yaml", "library/P.yaml"]),
                    true
                ),
                (agent, attached(false, &["library/Q.yaml"]), true),
            ],
            "each write goes through the port, from inside a change"
        );
        assert_eq!(attach.applied, strings(&["library/R.yaml"]));
        assert_eq!(
            attach.view.presets,
            strings(&["library/P.yaml", "library/Q.yaml", "library/R.yaml"]),
            "the answer is the attachments as committed"
        );
        assert_eq!(
            reorder.presets,
            strings(&["library/Q.yaml", "library/P.yaml"])
        );
        assert_eq!(
            (detach.default_preset, detach.presets),
            (false, strings(&["library/Q.yaml"]))
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
            &strings(&["engram/default"]),
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
            .detach_preset(agent, &strings(&["engram/default"]), true, Some(current))
            .is_ok());
    }

    // ── attach ──────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn attach_applies_what_fits_skips_the_rest_with_a_reason_and_places_after_the_anchor() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], false);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let answer = w
            .svc
            .attach_preset(
                agent,
                &strings(&[
                    "library/R.yaml",
                    "engram/default",
                    "library/S.yaml",
                    "library/Q.yaml",
                    "library/rules/a.md",
                    "engram/priming",
                    "library/M.yaml",
                    "../x.yaml",
                    "engram/default",
                    "library/R.yaml",
                ]),
                Some("library/P.yaml"),
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
            strings(&["library/R.yaml", "engram/default", "library/S.yaml"])
        );
        assert_eq!(
            skips,
            [
                ("library/Q.yaml", ALREADY_PRESENT),
                ("library/rules/a.md", WRONG_KIND),
                ("engram/priming", WRONG_KIND),
                ("library/M.yaml", NOT_FOUND),
                ("../x.yaml", ITEM_PATH_INVALID),
                ("engram/default", ALREADY_PRESENT),
                ("library/R.yaml", ALREADY_PRESENT),
            ]
        );
        assert_eq!(
            w.attachments(agent),
            attached(
                true,
                &[
                    "library/P.yaml",
                    "library/R.yaml",
                    "library/S.yaml",
                    "library/Q.yaml"
                ]
            ),
            "placed after the anchor in call order"
        );
        assert_eq!(
            answer.view.presets,
            strings(&[
                "library/P.yaml",
                "library/R.yaml",
                "library/S.yaml",
                "library/Q.yaml"
            ])
        );
    }

    #[test]
    fn attach_without_an_anchor_in_the_list_appends() {
        let profile = claude(&["library/P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        w.svc
            .attach_preset(
                agent,
                &strings(&["library/Q.yaml", "library/R.yaml"]),
                Some("library/X.yaml"),
                None,
            )
            .unwrap();

        assert_eq!(
            w.attachments(agent),
            attached(
                true,
                &["library/P.yaml", "library/Q.yaml", "library/R.yaml"]
            )
        );
    }

    // ── detach ──────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn detaching_the_default_preset_needs_yes_while_the_slot_is_on() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let without_yes = w.svc.detach_preset(
            agent,
            &strings(&["library/P.yaml", "engram/default"]),
            false,
            None,
        );
        let (class, reason) = refused(without_yes);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, DEFAULT_PRESET_DETACH)
        );
        assert_eq!(reason.args, json!({ "agent": agent.to_string() }));
        assert!(w.port.writes().is_empty(), "the refusal applied nothing");
        assert_eq!(
            w.attachments(agent),
            attached(true, &["library/P.yaml", "library/Q.yaml"])
        );

        let with_yes = w
            .svc
            .detach_preset(agent, &strings(&["engram/default"]), true, None)
            .unwrap();
        assert!(!with_yes.default_preset);
        let again = w
            .svc
            .detach_preset(
                agent,
                &strings(&["engram/default", "library/P.yaml"]),
                false,
                None,
            )
            .unwrap();
        assert!(
            !again.default_preset,
            "with the slot off there is nothing to confirm"
        );
        assert_eq!(w.attachments(agent), attached(false, &["library/Q.yaml"]));
        let idempotent = w
            .svc
            .detach_preset(agent, &strings(&["library/P.yaml"]), false, None)
            .unwrap();
        assert_eq!(idempotent.presets, strings(&["library/Q.yaml"]));
    }

    #[test]
    fn detach_with_a_spelling_that_is_not_a_reference_refuses_the_whole_call() {
        let profile = claude(&["library/P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();

        let refusal = w.svc.detach_preset(
            agent,
            &strings(&["library/P.yaml", "../x.yaml"]),
            true,
            None,
        );

        assert_eq!(
            code(refusal),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID.to_owned())
        );
        assert!(w.port.writes().is_empty());
    }

    // ── baseRev and reorder ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_agent_base_rev_is_the_sha256_of_the_canonical_attachments_and_order_sensitive() {
        let ab = claude(&["library/A.yaml", "library/B.yaml"], true);
        let ba = claude(&["library/B.yaml", "library/A.yaml"], true);
        let ab_off = claude(&["library/A.yaml", "library/B.yaml"], false);
        let ids = [ab.id, ba.id, ab_off.id];
        let w = World::new(vec![ab, ba, ab_off]);
        let revs: Vec<String> = ids
            .iter()
            .map(|&id| w.svc.agent_presets(id).unwrap().base_rev)
            .collect();

        assert_eq!(
            revs[0],
            sha256_of(r#"{"default":true,"presets":["library/A.yaml","library/B.yaml"]}"#)
        );
        assert_eq!(
            revs[1],
            sha256_of(r#"{"default":true,"presets":["library/B.yaml","library/A.yaml"]}"#)
        );
        assert_eq!(
            revs[2],
            sha256_of(r#"{"default":false,"presets":["library/A.yaml","library/B.yaml"]}"#)
        );
    }

    #[test]
    fn reorder_refuses_a_base_rev_the_registry_moved_past() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let shown = w.svc.agent_presets(agent).unwrap().base_rev;
        // Another writer moves the attachments after the view was read.
        assert!(w.registry.update_with(agent, |p| p.default_preset = false));

        let (class, reason) = refused(w.svc.reorder_presets(
            agent,
            &strings(&["library/Q.yaml", "library/P.yaml"]),
            &shown,
            None,
        ));

        let current =
            sha256_of(r#"{"default":false,"presets":["library/P.yaml","library/Q.yaml"]}"#);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, REV_MISMATCH)
        );
        assert_eq!(reason.args, json!({ "rev": current }));
        assert!(w.port.writes().is_empty());
        let view = w
            .svc
            .reorder_presets(
                agent,
                &strings(&["library/Q.yaml", "library/P.yaml"]),
                &current,
                None,
            )
            .unwrap();
        assert_eq!(view.presets, strings(&["library/Q.yaml", "library/P.yaml"]));
    }

    #[test]
    fn reorder_refuses_an_order_that_is_not_a_permutation() {
        let profile = claude(
            &["library/P.yaml", "library/Q.yaml", "library/P.yaml"],
            true,
        );
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        let rev = w.svc.agent_presets(agent).unwrap().base_rev;

        for order in [
            &["library/P.yaml", "library/Q.yaml"][..],
            &["library/P.yaml", "library/Q.yaml", "library/Q.yaml"],
            &["library/P.yaml", "library/Q.yaml", "engram/default"],
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
                json!({ "expected": ["library/P.yaml", "library/Q.yaml", "library/P.yaml"] })
            );
        }
        assert!(w.port.writes().is_empty());
        let view = w
            .svc
            .reorder_presets(
                agent,
                &strings(&["library/Q.yaml", "library/P.yaml", "library/P.yaml"]),
                &rev,
                None,
            )
            .unwrap();
        assert_eq!(
            view.presets,
            strings(&["library/Q.yaml", "library/P.yaml", "library/P.yaml"])
        );
        assert!(view.default_preset, "the Default slot is untouched");
    }

    #[test]
    fn the_base_rev_is_read_from_the_registry_inside_the_change() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], true);
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
                svc.reorder_presets(
                    agent,
                    &strings(&["library/Q.yaml", "library/P.yaml"]),
                    &shown,
                    None,
                )
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
        let current =
            sha256_of(r#"{"default":false,"presets":["library/P.yaml","library/Q.yaml"]}"#);
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, REV_MISMATCH)
        );
        assert_eq!(reason.args, json!({ "rev": current }));
        assert!(w.port.writes().is_empty());
    }

    #[test]
    fn detach_drops_an_attached_entry_that_is_not_a_reference_by_its_text() {
        let profile = claude(&["../bad.yaml", "library/P.yaml"], true);
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
        assert_eq!(view.presets, strings(&["library/P.yaml"]));
        assert_eq!(w.attachments(agent), attached(true, &["library/P.yaml"]));
        assert!(view.problems.is_empty(), "{:?}", view.problems);
    }

    // ── the port cell and the port's own refusals ───────────────────────────────────────────────────────────

    #[test]
    fn an_empty_or_dead_port_cell_answers_profile_edit_unavailable_and_changes_nothing() {
        let unplugged = World::make(vec![claude(&["library/P.yaml"], true)], true, false, true);
        let mut dead = World::new(vec![claude(&["library/P.yaml"], true)]);
        dead.port = Arc::new(Port::default());

        for w in [&unplugged, &dead] {
            w.library();
            let agent = w.registry.list()[0].id;
            let before = w.attachments(agent);
            let answers = [
                code(w.svc.agent_presets(agent)),
                code(
                    w.svc
                        .attach_preset(agent, &strings(&["library/Q.yaml"]), None, None),
                ),
                code(
                    w.svc
                        .detach_preset(agent, &strings(&["library/P.yaml"]), true, None),
                ),
                code(
                    w.svc
                        .reorder_presets(agent, &strings(&["library/P.yaml"]), "x", None),
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
        let profile = claude(&["library/P.yaml"], true);
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
                .detach_preset(agent, &strings(&["library/P.yaml"]), true, None);

            let Err(LibraryRefusal::Store(store)) = answer else {
                panic!("not passed through: {answer:?}");
            };
            assert_eq!(store.to_string(), shown, "unchanged");
            let error = store_command_error(&store, String::new());
            assert_eq!((error.code(), error.retry()), (bus, retry), "{shown}");
            assert_eq!(w.attachments(agent), attached(true, &["library/P.yaml"]));
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
                    .attach_preset(missing, &strings(&["engram/default"]), None, None),
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
                    .attach_preset(agent, &strings(&["engram/default"]), None, None)
            ),
            (RefusalClass::NotFound, NOT_FOUND.to_owned())
        );
    }

    // ── no mount ────────────────────────────────────────────────────────────────────────────────────────────

    // ADR-0400 decision 4: with no mount registered a library preset names no mount — absent, so it is skipped
    //   `not-found`; nothing refuses the call.
    #[test]
    fn with_no_mount_an_attach_skips_a_library_preset_as_not_found_and_the_other_edits_work() {
        let profile = claude(&["library/P.yaml", "library/Q.yaml"], false);
        let agent = profile.id;
        let w = World::make(vec![profile], false, true, true);
        w.library();
        assert_eq!(w.svc.status().root, None);

        let library = w
            .svc
            .attach_preset(agent, &strings(&["library/R.yaml"]), None, None)
            .unwrap();
        assert!(library.applied.is_empty());
        let skips: Vec<(&str, &str)> = library
            .skipped
            .iter()
            .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
            .collect();
        assert_eq!(skips, [("library/R.yaml", NOT_FOUND)]);

        let default = w
            .svc
            .attach_preset(agent, &strings(&["engram/default"]), None, None)
            .unwrap();
        let rev = default.view.base_rev.clone();
        let reorder = w
            .svc
            .reorder_presets(
                agent,
                &strings(&["library/Q.yaml", "library/P.yaml"]),
                &rev,
                None,
            )
            .unwrap();
        let detach = w
            .svc
            .detach_preset(agent, &strings(&["library/Q.yaml"]), false, None)
            .unwrap();

        assert_eq!(default.applied, strings(&["engram/default"]));
        assert_eq!(
            reorder.presets,
            strings(&["library/Q.yaml", "library/P.yaml"])
        );
        assert_eq!(w.attachments(agent), attached(true, &["library/P.yaml"]));
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
        let profile = claude(&["library/P.yaml"], true);
        let agent = profile.id;
        let w = World::new(vec![profile]);
        w.library();
        assert!(w.svc.agent_presets(agent).unwrap().problems.is_empty());
        fs::remove_dir_all(w.base.join("lib")).unwrap();

        let detach = w
            .svc
            .detach_preset(agent, &strings(&["engram/default"]), true, None)
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
        let profile = claude(&["library/P.yaml"], false);
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
            .detach_preset(agent, &strings(&["library/Q.yaml"]), false, None)
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
            .detach_preset(agent, &strings(&["library/Q.yaml"]), false, None)
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
        let profile = claude(&["library/P.yaml", "library/M.yaml"], true);
        let agent = profile.id;
        let mut shell = claude(&["library/P.yaml"], true);
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
                &strings(&["library/P.yaml", "library/M.yaml"]),
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
                BlockReason::Missing { item, .. } if item == "library/M.yaml"
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
        assert_eq!(shell_view.presets, strings(&["library/P.yaml"]));
    }

    // ── the `agent.new {presets}` judgement ─────────────────────────────────────────────────────────────────

    /// The judgement is `attach`'s over a profile outside the registry: the same attachments and the same skips an
    /// attach of the same presets to an identical registered agent gives — and it writes nothing (no port write, the
    /// profile not added to the registry).
    #[test]
    fn the_new_agent_judgement_is_attachs_over_a_profile_outside_the_registry() {
        let presets = strings(&[
            "library/R.yaml",
            "engram/default",
            "library/rules/a.md",
            "engram/priming",
            "library/M.yaml",
            "../x.yaml",
            "library/S.yaml",
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
        assert_eq!(
            judged.attachments,
            attached(true, &["library/R.yaml", "library/S.yaml"])
        );
    }

    /// With no mount registered, a library preset names no mount — absent, skipped `not-found` (ADR-0400 decision 4) —
    /// while `engram/default` alone is judged without the library.
    #[test]
    fn the_new_agent_judgement_with_no_mount_skips_a_library_preset_and_judges_the_default_alone() {
        let w = World::make(Vec::new(), false, true, true);
        let new = claude(&[], true);

        let library = w
            .svc
            .judge_new_presets(&new, &strings(&["library/R.yaml"]), None)
            .unwrap();
        assert_eq!(library.attachments, attached(true, &[]));
        assert_eq!(
            library
                .skipped
                .iter()
                .map(|skip| (skip.item.as_str(), skip.reason.code.as_str()))
                .collect::<Vec<_>>(),
            [("library/R.yaml", NOT_FOUND)]
        );

        let default = w
            .svc
            .judge_new_presets(&new, &strings(&["engram/default", "engram/priming"]), None)
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
                ("engram/default", ALREADY_PRESENT),
                ("engram/priming", WRONG_KIND)
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
            &strings(&["library/R.yaml"]),
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
                    .judge_new_presets(&new, &strings(&["engram/default"]), Some(current + 1))
            ),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE.to_owned())
        );
        let judged = w
            .svc
            .judge_new_presets(&new, &strings(&["library/R.yaml"]), Some(current))
            .unwrap();
        assert_eq!(judged.attachments, attached(true, &["library/R.yaml"]));
    }

    /// With no library service, a library preset refuses the whole judgement `library-unavailable` with the given
    /// reason, and the rest is judged as the service judges it.
    #[test]
    fn the_judgement_with_no_library_service_refuses_a_library_preset_and_judges_the_rest() {
        let new = claude(&[], false);

        let (class, reason) = refused(judge_new_presets_without_library(
            &new,
            &strings(&["engram/default", "library/R.yaml"]),
            "no service",
        ));
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
        );
        assert_eq!(reason.args, json!({ "message": "no service" }));

        let judged = judge_new_presets_without_library(
            &new,
            &strings(&["engram/default", "../x.yaml"]),
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
