//! # engram-dashboard-asset-library — the preset library
//!
//! The library holds instruction, skill, settings and preset files under one root folder. This crate reads, checks and
//! resolves them and makes the launch materials; the daemon creates the service, plugs its ports in and passes it values,
//! nothing more (ADR-0311).
//!
//! ## Dependency rule (invariant)
//!
//! - Among workspace crates exactly agent · platform · command · base (ADR-0311 decision 2). agent defines the ports this
//!   crate implements or calls; agent does not know this crate. This crate calls no daemon internals.
//! - No test feature (ADR-0311 decision 4): fakes are `#[cfg(test)]` inside the crate.
//! - No network client, no process runner — settings are not schema-checked, so nothing is fetched or probed
//!   (ADR-0359).
//! - No OS `cfg`: OS behaviour comes from platform (the platform gates 4 · 6 file lists do not name this crate).
//!
//! ## Backend extensibility (invariant — TRD S25 step-7 §1-1)
//!
//! Adding a backend to agent (its `AgentCommand` variant, backend word and settings check in the registry) must make
//! tags, raw settings fields, the selection of what an agent loads, the hand-over checks and the notices work with no
//! change here. So:
//!
//! - A backend tag and a raw-field name are strings, accepted only when the registry knows the word (ASCII
//!   case-insensitive). There is no backend enum here.
//! - The registry is reached only through [`backends::Backends`]; [`backends::AgentBackends`] is the only code here that
//!   calls agent's settings-check registry.
//! - Every per-backend difference comes from a port answer; the code never branches on a backend word, and no backend
//!   name appears in the source outside tests, doc comments describing today's port answers and built-in content.
//!
//! ## Gates
//!
//! `.github/workflows/ci.yml` holds the commands and expected values (canonical — not copied here), as steps
//! `asset-library gate 1`–`4` (ADR-0311 decision 6 numbering; gate 3 is the shared messaging step): ① the test line
//! `cargo test -p engram-dashboard-asset-library` · ② the direct workspace dependency name set {agent · asset-library ·
//! base · command · platform} and no direct `tauri*` package · ③ `asset_library` in the messaging isolation regex ·
//! ④ zero `use tauri` lines under `src/`.
//!
//! ## The service
//!
//! [`LibraryService`] owns the catalog behind the ADR-0324 gate (`gate.rs` — the lock order and the ticket rules are
//! there). Library files under the root are read with the store directly (local by assumption). Every stat, list,
//! canonicalize or read of a folder outside the library — folder entries, native skill roots, the working folder, the
//! library root itself — goes through [`folder::BoundedIo`], never under the library lock. The built-ins
//! (`Builtins::new`), the launch folder (`prepare_in`, the sweep, `release`), `library.json` (`config.rs`) and the
//! folder `set_root {create}` or the first start creates use std `fs`.
//!
//! ## The root
//!
//! The root is one setting persisted in `library.json` (`config.rs` — ADR-0375 · ADR-0376), read by the daemon before
//! [`LibraryService::new`]; `new` does no disk work beyond the built-ins. [`LibraryService::set_root`] and
//! [`LibraryService::first_start`] are the only writers of the setting. Each publishes the root through
//! [`LibraryService::change_root`] first and writes `library.json` only after that succeeded (ADR-0381): a refused root
//! change writes nothing, and a write that fails leaves the new root live and the setting `unsaved` — nothing is undone.
//! One root-change mutex serializes them across the root change and the write; under it they take only the gate's locks
//! and the config's state mutex, a leaf held for no IO.
//!
//! The root generation starts each run at the base the daemon draws (`LibraryValues::root_generation_base`), so a
//! generation kept from an earlier run does not match; generations are compared only for equality (ADR-0381).
//!
//! ## Refusals
//!
//! A body refuses with [`LibraryRefusal`]: a [`RefusalClass`] plus an agent `launch::Reason`, or the profile store's
//! own error passed through. The command layer maps the class onto the bus code (ADR-0379); every refusal that
//! applied nothing must map onto a code that does not keep the request id.
// ADR-0311
// ADR-0324
// ADR-0379
// ADR-0381

pub mod backends;
pub mod builtin;
pub mod catalog;
pub mod config;
pub mod excerpt;
pub mod folder;
pub mod format;
mod gate;
pub mod launch;
pub mod path;
pub mod refs;
pub mod resolve;
pub mod store;
pub mod validate;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use engram_dashboard_agent::launch::{backend_word, escape_controls, BlockReason, Reason};
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, StoreError,
};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::{normalize_folder, path_key, same_path, PathKey};
use serde_json::json;

use crate::backends::Backends;
use crate::builtin::Builtins;
use crate::catalog::{
    key, preset_refs, Catalog, Item, ItemContent, ItemKind, ScanItem, ScanResult,
};
use crate::config::{ConfigFile, ConfigHealth, LibraryConfig, RootSetting};
use crate::folder::{BoundedIo, FolderStatus, LaunchIo, Need};
use crate::format::read::ITEM_PATH_INVALID;
use crate::gate::{Gate, Tags};
pub use crate::gate::{Numbers, State, NOT_FOUND};
use crate::path::{AssetRef, LibPath, PathError};
use crate::refs::{reference_count, referrers_of};
use crate::resolve::{resolve, CatalogLookup, Resolution, RootState, Target};
use crate::store::LibraryStore;

/// The largest library file a scan or the list safety net reads. A longer file is a failed read: its item is kept as
/// it was and the scan reports the error.
pub const LIBRARY_READ_CAP: u64 = 16 * 1024 * 1024;

/// The code of a path that lies at or below a link inside the library (`args` = `{path}`) — such a path is never read
/// or merged (U1).
// ADR-0365
// TRD S25 step-7 B8
pub const PATH_THROUGH_LINK: &str = "path-through-link";

/// The code of a catalog read that could not be answered because no full scan of the library could be applied — no
/// root is set, or the root could not be walked — and of block ④, a library preset attached while the root is unset or
/// cannot be used (`args` = `{message}`, the reason).
// TRD S25 step-7 B8
pub const LIBRARY_UNAVAILABLE: &str = "library-unavailable";

/// The code of an edit whose root generation is not the current one (`args` = `{given, current}`).
// ADR-0377
pub const ROOT_GENERATION_STALE: &str = "root-generation-stale";

/// The code of a `set_root` whose folder cannot be used — it could not be created, its stat did not answer `Ok`, or
/// its walk failed at the root; nothing published, `library.json` not written (`args` = `{path, why}`).
// ADR-0379
// ADR-0381
pub const LIBRARY_ROOT_UNUSABLE: &str = "library-root-unusable";

/// The code of a `set_root` while `library.json` is read-only for the run — newer or unreadable at load (`args` =
/// `{why}`).
// ADR-0376
pub const LIBRARY_CONFIG_READ_ONLY: &str = "library-config-read-only";

/// The code of a `set_root` whose `library.json` write was refused or failed after the new root was published —
/// published but not saved: the root stays live this run, the file keeps its setting, and the next start follows the
/// file that is left (`args` = `{why}`, which says so).
// ADR-0379
// ADR-0381
pub const LIBRARY_CONFIG_WRITE_FAILED: &str = "library-config-write-failed";

const MERGE_JOB: &str = "library-merge";

/// The class of a [`LibraryRefusal`] — the command layer maps it onto the bus code: `Unavailable` onto `CONFLICT`,
/// the others onto their namesakes (ADR-0379 decision 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalClass {
    Conflict,
    NotFound,
    InvalidArgument,
    Unavailable,
    Internal,
}

/// Why a body refused. A refusal applied nothing to the library or the registry — folders a `set_root {create}` made
/// before it refused [`LIBRARY_ROOT_UNUSABLE`] do not count as applied (ADR-0379 decision 4) — except
/// [`LIBRARY_CONFIG_WRITE_FAILED`], which comes after the new root was published and still maps onto a code that does
/// not keep the request id, so a retry saves again (ADR-0381).
#[derive(Debug)]
pub enum LibraryRefusal {
    Refused {
        class: RefusalClass,
        reason: Reason,
    },
    /// The profile-edit port's store error, unchanged — answered as agent `store_command_error` answers every
    /// registry-writing command (ADR-0379 decision 2).
    Store(StoreError),
}

impl LibraryRefusal {
    fn new(class: RefusalClass, code: &str, args: serde_json::Value) -> Self {
        LibraryRefusal::Refused {
            class,
            reason: Reason {
                code: code.to_owned(),
                args,
                line: None,
            },
        }
    }

    fn unavailable(why: String) -> Self {
        Self::new(
            RefusalClass::Unavailable,
            LIBRARY_UNAVAILABLE,
            json!({ "message": why }),
        )
    }
}

/// What the daemon hands the service.
pub struct LibraryValues {
    pub registry: Arc<ProfileRegistry>,
    /// `AgentBackends` in production.
    pub backends: Arc<dyn Backends>,
    /// `library.json` as `config::load` read it — read only by `new`; after that the service's own copy is live.
    pub config: LibraryConfig,
    /// The place the root setting's default value names this run (ADR-0375) — also `status`'s `suggested_new`.
    pub default_root: PathBuf,
    /// The daemon identity (`status`), owned by the daemon.
    pub daemon_id: String,
    /// The root generation this run starts at — drawn by the daemon at random below 2^52, so a generation kept from an
    /// earlier run does not match; compared only for equality (ADR-0381).
    pub root_generation_base: u64,
    /// The built-in assets folder (`builtin::Builtins`) — read once by `new`; `None` = there is none.
    pub builtin_dir: Option<PathBuf>,
    /// The volatile launch folder — written by the launch materials (step-7 P9).
    pub launch_root: PathBuf,
    /// The home folder handed to the backend's native skill roots (step-7 P9).
    pub home: Option<PathBuf>,
    /// The daemon's environment; a profile's env applies on top per agent (step-7 P9).
    pub env: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

/// The library service: the catalog, its scans and its reads (module header · `gate.rs`).
pub struct LibraryService {
    values: LibraryValues,
    store: Arc<dyn LibraryStore>,
    io: BoundedIo,
    gate: Arc<Gate>,
    builtins: Builtins,
    config: ConfigFile,
    /// Serializes `set_root` and the first start (module header).
    root_change: Mutex<()>,
    /// How many root-change takes found it held — tests observe a waiter.
    #[cfg(test)]
    root_change_blocked: std::sync::atomic::AtomicUsize,
}

/// The answer of [`LibraryService::status`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryStatus {
    pub daemon_id: String,
    /// The published root; `None` = unset.
    pub root: Option<PathBuf>,
    /// `None` = `library.json` could not be read (`config` is read-only or corrupt); while `config` is unsaved, the
    /// live root's setting.
    pub root_setting: Option<RootSettingKind>,
    pub state: RootStatus,
    pub config: ConfigHealth,
    /// The default place.
    pub suggested_new: PathBuf,
    /// The value an edit may carry ([`LibraryService::change_at`]).
    pub root_generation: u64,
}

/// What the root setting holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootSettingKind {
    /// The default value — `library.json` says `"default"` or is not written yet.
    Default,
    /// A path the user set.
    Explicit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootStatus {
    /// No root is published.
    Unset,
    Ok,
    /// The root's stat answered anything but `Ok` — missing, not a folder, unreadable, timed out.
    Missing {
        why: String,
    },
}

/// The answer of [`LibraryService::first_start`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstStart {
    /// `library.json` was not missing at load, or the step already published this run — nothing done. A `Failed`
    /// step stays due, so a second call in the same run would retry.
    NotDue,
    /// The default place was created, the root published and then the setting written at its default value.
    Published,
    /// The default root was published but `library.json` was not written (`why`): live this run, `config: unsaved`;
    /// the next start follows the file that is left (with none left, the first start runs again).
    // ADR-0381
    Unsaved(String),
    /// The default place could not be created or the root change refused: nothing published or written, the root
    /// unset; the next start tries again.
    Failed(String),
}

/// The answer of [`LibraryService::folders`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldersAnswer {
    /// Every folder of the library, by path; a skill folder is marked.
    pub folders: Vec<FolderNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub skill: bool,
}

/// The answer of [`LibraryService::read`].
#[derive(Clone, Debug, PartialEq)]
pub struct ReadAnswer {
    /// Kind, name, state, description and reference count as `list` shows it.
    pub row: ListRow,
    /// The item on disk under the published root.
    pub absolute: PathBuf,
    /// Presets that list it.
    pub presets_direct: Vec<String>,
    /// Presets that reach it only through nested presets.
    pub presets_indirect: Vec<String>,
    /// Agents that attach it (a preset only).
    pub agents_direct: Vec<AgentId>,
    /// Agents that attach a preset reaching it, and do not attach it.
    pub agents_indirect: Vec<AgentId>,
    /// An instruction's excerpt (`excerpt::EXCERPT_CHARS`).
    pub excerpt: Option<String>,
    /// A settings asset's raw-field names, in file order.
    pub settings_fields: Vec<String>,
}

/// What one read computation resolves, before the root state is known.
struct Want {
    backend: Option<String>,
    mode: Option<AgentOutputFormat>,
    default_preset: bool,
    presets: Vec<AssetRef>,
    unparsed: Vec<(String, PathError)>,
}

/// The answer of [`LibraryService::list`].
#[derive(Clone, Debug, PartialEq)]
pub struct ListAnswer {
    pub folder: Option<LibPath>,
    /// The items directly in the folder, by path — a skill folder is an item (kind = skill).
    pub rows: Vec<ListRow>,
    /// The plain folders directly in the folder (not skill folders), by path.
    pub folders: Vec<FolderRow>,
    /// The built-in rows — always empty for now; what fills them is not decided here.
    pub builtins: Vec<ListRow>,
}

/// One plain subfolder line of a [`ListAnswer`].
#[derive(Clone, Debug, PartialEq)]
pub struct FolderRow {
    pub path: String,
    /// Its last name.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub path: String,
    pub kind: ItemKind,
    /// A skill's name; every other item's file name.
    pub name: String,
    /// A broken file's reason, a settings asset's first refusal (in any backend's field), or [`PATH_THROUGH_LINK`];
    /// `None` = it reads.
    pub state: Option<Reason>,
    /// Presets that reach it plus agents that attach it (`refs::reference_count`).
    pub references: usize,
    /// An instruction's excerpt, a skill's description, a preset's or settings asset's `description`; else empty.
    pub description: String,
}

/// The answer of [`LibraryService::refresh`] — over the folder asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshReport {
    /// Items inserted, replaced or removed (a skill counts its folder and its `SKILL.md`).
    pub changed: usize,
    /// The problems of the files re-read: a broken file's reason, a preset's line problems and the references it lists
    /// that the catalog does not hold ([`NOT_FOUND`]), a settings asset's refusals.
    pub problems: Vec<(LibPath, Reason)>,
    /// Re-read `.yaml` / `.yml` files that declare no `kind` — not listed.
    pub no_kind: Vec<LibPath>,
    /// Links below the root that were not entered.
    // ADR-0365
    pub skipped_links: Vec<LibPath>,
    /// Where the walk or a read failed (`None` = the root), with the error; those items are kept as they were. When
    /// nothing could be scanned (no root, the root unreadable, or a refresh's own scan of a folder with a dot or temp
    /// name), the one entry is `(None, why)`.
    pub errors: Vec<(Option<LibPath>, String)>,
}

/// What one path on disk is, read the way the scan admits paths.
enum Observed {
    /// The items read at or below it (none when it is gone).
    Items(Vec<Item>),
    /// It is at or below a link inside the library — not read.
    ThroughLink,
    /// Its name is not admitted (a dot name, a temp name) — not read.
    Refused,
}

impl LibraryService {
    /// `io` bounds every folder operation outside the library and spawns the service's short-lived jobs. The built-ins
    /// are read here, once.
    pub fn new(values: LibraryValues, store: Arc<dyn LibraryStore>, io: BoundedIo) -> Self {
        let root = ConfigFile::initial_root(&values.config, &values.default_root);
        let gate = Arc::new(Gate::new(root, values.root_generation_base));
        let builtins = Builtins::new(values.builtin_dir.as_deref(), &*values.backends);
        let config = ConfigFile::new(&values.config);
        LibraryService {
            values,
            store,
            io,
            gate,
            builtins,
            config,
            root_change: Mutex::new(()),
            #[cfg(test)]
            root_change_blocked: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The built-in priming text read when the service was made — what a launch whose plan includes the priming merges
    /// in first. Empty for its stand-in; then no plan includes it, and the launch check tells `builtin-unavailable` when
    /// the preset reaches `engram:/priming` (a preset that does not list it includes no priming and tells nothing).
    pub fn builtin_priming_text(&self) -> &str {
        self.builtins.priming_text()
    }

    /// One library preset on its own (the edit area — no agent: everything is shown, no format verdict), after the
    /// first-scan entrance, over the catalog under one read lock; its folder entries are stat'ed afterwards, outside
    /// the lock, on one read-computation budget. A first scan that could not be applied is block ④ with its reason.
    pub fn resolve_preset(&self, preset: &LibPath) -> Resolution {
        let want = Want {
            backend: None,
            mode: None,
            default_preset: false,
            presets: vec![AssetRef::Library(preset.clone())],
            unparsed: Vec::new(),
        };
        self.resolve_wants(&[&want]).remove(0)
    }

    /// What agent `id` gets ([`LibraryService::resolve_agents`] for one agent).
    pub fn resolve_agent(&self, id: AgentId) -> Option<Resolution> {
        self.resolve_agents(&[id])
            .pop()
            .and_then(|(_, resolution)| resolution)
    }

    /// What each agent gets (`agent.presets`, tree ⛔): one first-scan entrance, the root stat (when a library preset
    /// is attached), one read lock for all of them, then the folder stats — all on one read-computation budget, each
    /// distinct folder stat'ed once; then, with no lock, a link probe for each ① `not-found` library row. `None` = no
    /// such agent, or its backend has no settings check (it takes no preset — B13).
    pub fn resolve_agents(&self, ids: &[AgentId]) -> Vec<(AgentId, Option<Resolution>)> {
        let found: Vec<(AgentId, Option<Want>)> =
            ids.iter().map(|&id| (id, self.agent_want(id))).collect();
        let wants: Vec<&Want> = found.iter().filter_map(|(_, want)| want.as_ref()).collect();
        let mut resolved = self.resolve_wants(&wants).into_iter();
        found
            .into_iter()
            .map(|(id, want)| (id, want.and_then(|_| resolved.next())))
            .collect()
    }

    /// One resolution per want, in order. Locks: the first-scan entrance, a brief read lock for the root, the root stat
    /// (only when a library preset is attached somewhere — otherwise it decides nothing) with no lock, one read lock for
    /// every resolution, the folder stats with no lock, then the link probes of `mark_links` with no lock. A block-④
    /// resolution has no folder and no `not-found` row, so it is neither stat'ed nor probed.
    fn resolve_wants(&self, wants: &[&Want]) -> Vec<Resolution> {
        let joined = self.first_scan_join();
        let root = self.gate.with_read(|state| state.root.clone());
        let io = self.io.begin_read();
        let library_attached = wants
            .iter()
            .flat_map(|want| &want.presets)
            .any(|preset| matches!(preset, AssetRef::Library(_)));
        let root_state = self.root_state(&io, root.as_deref(), joined, library_attached);
        let targets: Vec<Target> = wants
            .iter()
            .map(|want| Target {
                backend: want.backend.clone(),
                mode: want.mode,
                default_preset: want.default_preset,
                presets: want.presets.clone(),
                unparsed: want.unparsed.clone(),
                root: root_state.clone(),
            })
            .collect();
        let backends = &*self.values.backends;
        let mut resolved: Vec<Resolution> = self.gate.with_read(|state| {
            let lookup = CatalogLookup {
                catalog: &state.catalog,
                root: state.root.as_deref(),
                builtins: &self.builtins,
            };
            targets
                .iter()
                .map(|target| resolve(target, &lookup, backends))
                .collect()
        });
        let folders: Vec<(PathBuf, Need)> = resolved
            .iter()
            .flat_map(Resolution::folder_paths)
            .map(|path| (path, Need::Required))
            .collect();
        let statuses = self.io.stat_all(&io, &folders);
        for resolution in &mut resolved {
            resolution.attach_folder_status(&statuses);
            if let Some(root) = &root {
                self.mark_links(root, resolution);
            }
        }
        resolved
    }

    /// The root as block ④ needs it: unset; unusable because no full scan could be applied (`joined`); or, when a
    /// library preset is attached (otherwise the root decides nothing and is not asked), stat'ed as a required ask on
    /// `io`.
    fn root_state(
        &self,
        io: &LaunchIo,
        root: Option<&Path>,
        joined: Result<(), String>,
        library_attached: bool,
    ) -> RootState {
        match (root, joined) {
            (None, _) => RootState::Unset,
            (Some(_), Err(why)) => RootState::Unavailable(why),
            (Some(root), Ok(())) if library_attached => match self.stat_root(io, root) {
                Ok(()) => RootState::Ok,
                Err(why) => RootState::Unavailable(why),
            },
            (Some(_), Ok(())) => RootState::Ok,
        }
    }

    /// The root stat'ed as a required ask on `io`; `Err(why)` = anything but `Ok`.
    fn stat_root(&self, io: &LaunchIo, root: &Path) -> Result<(), String> {
        let status = self
            .io
            .stat_all(io, &[(root.to_path_buf(), Need::Required)])
            .remove(&path_key(root))
            .expect("stat_all answers every path it is asked");
        match status {
            FolderStatus::Ok => Ok(()),
            FolderStatus::Missing => Err("missing".to_owned()),
            FolderStatus::NotADirectory => Err("not a folder".to_owned()),
            FolderStatus::Unreadable(why) => Err(why),
        }
    }

    /// Turns each ① `not-found` row of a library reference that lies at or below a link inside the library into ②
    /// [`PATH_THROUGH_LINK`] (R11): the scan never enters such a link, so the catalog lacks the item. Asks the store's
    /// `first_link` per row — call it outside every lock.
    fn mark_links(&self, root: &Path, resolution: &mut Resolution) {
        for problem in &mut resolution.problems {
            let BlockReason::Missing { item, via, reason } = problem else {
                continue;
            };
            let Ok(AssetRef::Library(path)) = AssetRef::parse(item) else {
                continue;
            };
            if reason.code != NOT_FOUND {
                continue;
            }
            match self.store.first_link(root, &path) {
                Ok(Some(_)) => {
                    let through = BlockReason::Broken {
                        item: item.clone(),
                        via: via.clone(),
                        reason: Reason {
                            code: PATH_THROUGH_LINK.to_owned(),
                            args: json!({ "path": item }),
                            line: reason.line,
                        },
                    };
                    *problem = through;
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(
                    path = path.as_str(),
                    "first_link 실패, not-found 행을 그대로 둠: {e}"
                ),
            }
        }
    }

    /// What agent `id` attaches, read from the registry (outside every library lock).
    fn agent_want(&self, id: AgentId) -> Option<Want> {
        self.want_of(&self.values.registry.get(id)?)
    }

    /// What `profile` attaches; `None` = its backend has no settings check (it takes no preset — B13).
    fn want_of(&self, profile: &AgentProfile) -> Option<Want> {
        let backend = backend_word(&profile.command);
        self.values.backends.for_word(&backend)?;
        let mut presets = Vec::new();
        let mut unparsed = Vec::new();
        for attached in &profile.presets {
            match AssetRef::parse(&attached.path) {
                Ok(preset) => presets.push(preset),
                Err(why) => unparsed.push((attached.path.clone(), why)),
            }
        }
        Some(Want {
            backend: Some(backend),
            mode: Some(output_format(&profile.command)),
            default_preset: profile.default_preset,
            presets,
            unparsed,
        })
    }

    /// The items and plain subfolders directly in `folder` (`None` = the root), after the first-scan entrance, under
    /// one read lock — no scan once the first scan is raised. `Err` = the first scan could not be applied
    /// ([`LIBRARY_UNAVAILABLE`] — ADR-0324 decision 6: that failure is the answer). A `folder` the catalog does not
    /// know is checked on disk alone and merged (the safety net — a late merge, ADR-0324 decision 5); one at or below
    /// a link answers a single [`PATH_THROUGH_LINK`] row; one with a dot or temp name answers no rows. A folder the
    /// catalog does not know answers its items only — its subfolder lines appear after a refresh.
    pub fn list(&self, folder: Option<&LibPath>) -> Result<ListAnswer, Reason> {
        self.first_scan_join().map_err(|why| Reason {
            code: LIBRARY_UNAVAILABLE.to_owned(),
            args: json!({ "message": why }),
            line: None,
        })?;
        // ADR-0324 decision 5: the numbers are noted in the read lock that reads the root the disk read uses, so a root
        //   change published after it moves the root generation and discards the merge.
        let known = self.gate.with_read(|state| match folder {
            Some(path) if !is_known(&state.catalog, path) => {
                Err((path, state.root.clone(), self.gate.numbers()))
            }
            _ => Ok(self.answer(state, folder)),
        });
        let (path, root, noted) = match known {
            Ok(answer) => return Ok(answer),
            Err(unknown) => unknown,
        };
        if let Some(root) = root {
            match self.observe(&root, path) {
                Observed::ThroughLink => {
                    return Ok(ListAnswer {
                        folder: folder.cloned(),
                        rows: vec![through_link_row(path)],
                        folders: Vec::new(),
                        builtins: Vec::new(),
                    })
                }
                Observed::Items(items) => {
                    self.gate.late_merge(noted, |catalog, may_insert| {
                        catalog.apply_items(items, may_insert)
                    });
                }
                Observed::Refused => {}
            }
        }
        Ok(self.gate.with_read(|state| self.answer(state, folder)))
    }

    /// Re-scans `folder` (`None` = the whole library) on its own ticket, or joins a covering scan queued and not yet
    /// walking (ADR-0324 decision 7). Before the first scan is raised a folder refresh passes through a full scan
    /// (decision 6): it joins a queued one or runs its own and takes its share; if a full scan (or a first-start root
    /// change) is already walking, it waits for it and then scans its folder like any refresh — or, when what it
    /// waited for failed, goes through a full scan once more.
    pub fn refresh(&self, folder: Option<&LibPath>) -> RefreshReport {
        match self.gate.refresh(folder, &|root, range, tags| {
            self.build_scan(root, range, tags)
        }) {
            Ok(report) => report.share(folder),
            Err(why) => failed(why),
        }
    }

    /// The first-scan entrance every method that reads the catalog passes first, before any ticket or lock (ADR-0324
    /// decision 6). `Err` = no full scan could be applied, with the reason; the catalog then holds only what late
    /// merges put in. The launch check does not pass it.
    pub fn first_scan_join(&self) -> Result<(), String> {
        self.gate
            .first_scan_join(&|root, range, tags| self.build_scan(root, range, tags))
    }

    /// `f` over the catalog and the root under one read lock. `f` may read the profile registry and call pure backend
    /// port answers (no IO, no lock — TRD S25 step-7 §3-8 step 4) and nothing else outward; it must not call back into
    /// the service.
    pub fn with_read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.gate.with_read(f)
    }

    /// The numbers to note before a disk read whose result will be merged late ([`LibraryService::late_merge`]) — and
    /// before reading the root that disk read uses, or inside the same read lock ([`LibraryService::with_read`]):
    /// noted after the root was read, they may already include a root change the read does not see.
    pub fn numbers(&self) -> Numbers {
        self.gate.numbers()
    }

    /// The conditional late merge (ADR-0324 decision 5) — `f(catalog, may_insert)` under the write lock unless a
    /// generation moved since `noted`; `may_insert` is false when a scan was applied in between. `f` applies per item
    /// (`Catalog::apply_items`) and returns the touched keys. Positive observations only — a late merge never removes.
    pub fn late_merge(
        &self,
        noted: Numbers,
        f: impl FnOnce(&mut Catalog, bool) -> Vec<PathKey>,
    ) -> Vec<PathKey> {
        self.gate.late_merge(noted, f)
    }

    /// Hands a late merge of `items` (read after `noted`) to a short-lived job that takes the write lock normally, so
    /// the caller never waits on it and a contended lock delays it without dropping it (TRD S25 step-7 B26). The work
    /// sits in a take-once slot outside the job: if the job cannot be started (it is dropped unrun), the merge runs
    /// here instead, exactly once — so call it holding no library guard (the in-place merge takes the write lock).
    pub fn hand_off_merge(&self, noted: Numbers, items: Vec<Item>) {
        let slot = Arc::new(Mutex::new(Some(items)));
        let job = {
            let (slot, gate) = (Arc::clone(&slot), Arc::clone(&self.gate));
            Box::new(move || merge_from(&gate, noted, &slot))
        };
        if let Err(e) = self.io.spawner().spawn(MERGE_JOB, job) {
            tracing::warn!("library-merge spawn 실패, 호출 스레드에서 합침: {e}");
            merge_from(&self.gate, noted, &slot);
        }
    }

    /// A change inside Engram (TRD S25 step-7 §6 writer 5 — step 9a and 9b edits): its own ticket, then `f` under the
    /// write lock with the catalog, the root and the sequence; `f` writes per item and returns the touched keys; the
    /// change generation is raised before the lock is released. A caller that reads the catalog passes the first-scan
    /// entrance before this call. The change's disk effect must be finished inside `f` (or before the call): a reader
    /// that notes the numbers after the raise assumes it sees the new disk.
    pub fn change<R>(
        &self,
        f: impl FnOnce(&mut Catalog, Option<&Path>, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> R {
        self.gate.change(f)
    }

    /// Moves the library to `new_root` (TRD S25 step-7 §6 writer 6 — step 9a): the new root is stat'ed first on one
    /// read-computation budget, outside the gate; then, exclusive behind every earlier ticket, it is walked with empty
    /// tags and published with a fresh catalog in one write lock. `Err` = refused, nothing published: the stat did not
    /// answer `Ok`, or the walk failed at the root.
    ///
    /// It neither records the root in `library.json` nor takes the root-change mutex: every caller outside this crate
    /// goes through [`LibraryService::set_root`] or [`LibraryService::first_start`] (tests excepted).
    pub fn change_root(&self, new_root: PathBuf) -> Result<RefreshReport, String> {
        let io = self.io.begin_read();
        self.stat_root(&io, &new_root)
            .map_err(|why| format!("the new library root cannot be used: {why}"))?;
        let walk = |root: &Path| self.build_scan(root, None, &Tags::new());
        self.gate
            .change_root(new_root, walk)
            .map(|report| report.share(None))
    }

    /// [`LibraryService::change`] for an edit that carries the root generation it read from `status` (`None` = the
    /// current root). The generation is compared inside the change's own write-lock take, so an edit queued behind a
    /// root change sees it: a stale one refuses `Conflict` · [`ROOT_GENERATION_STALE`] before `f` runs, nothing
    /// applied.
    // ADR-0377
    pub fn change_at<R>(
        &self,
        root_generation: Option<u64>,
        f: impl FnOnce(&mut Catalog, Option<&Path>, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> Result<R, LibraryRefusal> {
        self.gate
            .change_checked(root_generation, f)
            .map_err(|current| {
                LibraryRefusal::new(
                    RefusalClass::Conflict,
                    ROOT_GENERATION_STALE,
                    json!({ "given": root_generation, "current": current }),
                )
            })
    }

    /// The root, its setting and `library.json` now. The root is stat'ed on one read budget outside every lock; the
    /// root and its generation are read in one read lock.
    pub fn status(&self) -> LibraryStatus {
        let (root, root_generation) = self
            .gate
            .with_read(|state| (state.root.clone(), self.gate.numbers().root));
        let state = match &root {
            None => RootStatus::Unset,
            Some(root) => match self.stat_root(&self.io.begin_read(), root) {
                Ok(()) => RootStatus::Ok,
                Err(why) => RootStatus::Missing { why },
            },
        };
        let (setting, config) = self.config.view();
        LibraryStatus {
            daemon_id: self.values.daemon_id.clone(),
            root,
            root_setting: setting.map(|setting| match setting {
                RootSetting::Default => RootSettingKind::Default,
                RootSetting::Explicit(_) => RootSettingKind::Explicit,
            }),
            state,
            config,
            suggested_new: self.values.default_root.clone(),
            root_generation,
        }
    }

    /// Moves the library to `path`, creating it first when `create`, and records it: in order the path is normalized
    /// (it must be absolute), the root-change mutex taken, a read-only `library.json` refused, the folder created, the
    /// root changed and — only after that succeeded — `library.json` written: the default value when `path` is the
    /// default place, else the path. Answers the status after the change.
    ///
    /// Refusals: a relative path `InvalidArgument` · `item-path-invalid`; a read-only `library.json` `Unavailable` ·
    /// [`LIBRARY_CONFIG_READ_ONLY`] before anything is created; a folder that cannot be created or used `Unavailable` ·
    /// [`LIBRARY_ROOT_UNUSABLE`] — nothing published or written, folders `create` made stay; a write refused or failed
    /// after the publish `Unavailable` · [`LIBRARY_CONFIG_WRITE_FAILED`] — the new root stays live, the file keeps its
    /// setting and `status` says `unsaved`.
    // ADR-0366
    // ADR-0375
    // ADR-0376
    // ADR-0381
    pub fn set_root(&self, path: &Path, create: bool) -> Result<LibraryStatus, LibraryRefusal> {
        let root = normalize_folder(path).map_err(|why| {
            LibraryRefusal::new(
                RefusalClass::InvalidArgument,
                ITEM_PATH_INVALID,
                json!({
                    "path": escape_controls(&path.display().to_string()),
                    "why": why.to_string(),
                }),
            )
        })?;
        let _serial = self.take_root_change();
        if let Some(why) = self.config.read_only() {
            return Err(LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_CONFIG_READ_ONLY,
                json!({ "why": why }),
            ));
        }
        if create {
            std::fs::create_dir_all(&root).map_err(|e| root_unusable(&root, e.to_string()))?;
        }
        self.change_root(root.clone())
            .map_err(|why| root_unusable(&root, why))?;
        let setting = if same_path(&root, &self.default_place()) {
            RootSetting::Default
        } else {
            RootSetting::Explicit(root.clone())
        };
        if let Err(why) = self.config.save(&setting) {
            return Err(LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_CONFIG_WRITE_FAILED,
                json!({
                    "why": format!(
                        "the library root {} is live but not saved; the next start follows library.json as it is: {why}",
                        root.display()
                    )
                }),
            ));
        }
        let status = self.status();
        tracing::info!(
            root = %root.display(),
            setting = ?status.root_setting,
            "library root set"
        );
        Ok(status)
    }

    /// The first-start step: when `library.json` was missing at load, creates the default place, publishes it as the
    /// root and only then writes the setting at its default value. A failed create or root change writes nothing and
    /// leaves the root unset, and the next start tries again; a failed write leaves the default root live this run
    /// (`unsaved`, logged), and the next start follows the file that is left. A configured root that vanished is never
    /// re-created (only a missing `library.json` makes the step due). The daemon calls it once, before the commands are
    /// served.
    // ADR-0312
    // ADR-0366
    // ADR-0375
    // ADR-0381
    pub fn first_start(&self) -> FirstStart {
        let _serial = self.take_root_change();
        if !self.config.first_start_due() {
            return FirstStart::NotDue;
        }
        let root = self.values.default_root.clone();
        let failed = |why: String| {
            tracing::warn!(root = %root.display(), "library first start failed; root unset: {why}");
            FirstStart::Failed(why)
        };
        if let Err(e) = std::fs::create_dir_all(&root) {
            return failed(format!("create failed: {e}"));
        }
        if let Err(why) = self.change_root(root.clone()) {
            return failed(why);
        }
        match self.config.save(&RootSetting::Default) {
            Ok(()) => {
                tracing::info!(root = %root.display(), "library first start: default root published");
                FirstStart::Published
            }
            Err(why) => {
                tracing::warn!(
                    root = %root.display(),
                    "library first start: default root live but not saved: {why}"
                );
                FirstStart::Unsaved(why)
            }
        }
    }

    /// The default place spelled as `set_root` compares paths.
    fn default_place(&self) -> PathBuf {
        normalize_folder(&self.values.default_root)
            .unwrap_or_else(|_| self.values.default_root.clone())
    }

    fn take_root_change(&self) -> MutexGuard<'_, ()> {
        #[cfg(test)]
        if matches!(
            self.root_change.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            self.root_change_blocked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        lock(&self.root_change)
    }

    /// The whole folder tree: every folder the catalog holds plus every skill folder, by path; a skill folder is
    /// marked. After the first-scan entrance, under one read lock; with no published root `Unavailable` ·
    /// [`LIBRARY_UNAVAILABLE`].
    pub fn folders(&self) -> Result<FoldersAnswer, LibraryRefusal> {
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        Ok(self.gate.with_read(|state| {
            let catalog = &state.catalog;
            let mut seen = std::collections::HashSet::new();
            let skills = catalog
                .items()
                .filter(|item| item.kind == ItemKind::Skill)
                .map(|item| &item.path);
            let mut folders: Vec<FolderNode> = catalog
                .folders()
                .chain(skills)
                .filter(|folder| seen.insert(key(folder)))
                .map(|folder| FolderNode {
                    path: folder.as_str().to_owned(),
                    name: folder.file_name().to_owned(),
                    skill: catalog
                        .get(&key(folder))
                        .is_some_and(|item| item.kind == ItemKind::Skill),
                })
                .collect();
            folders.sort_by(|a, b| a.path.cmp(&b.path));
            FoldersAnswer { folders }
        }))
    }

    /// One item's detail from the catalog (no disk read): its `list` row, its absolute path under the published root,
    /// who refers to it directly and indirectly, an instruction's excerpt and a settings asset's raw-field names.
    /// After the first-scan entrance; with no published root `Unavailable` · [`LIBRARY_UNAVAILABLE`]; a path the
    /// catalog does not hold `NotFound` · `not-found`.
    pub fn read(&self, path: &LibPath) -> Result<ReadAnswer, LibraryRefusal> {
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        self.gate.with_read(|state| {
            let catalog = &state.catalog;
            let Some(root) = state.root.as_deref() else {
                return Err(LibraryRefusal::unavailable(gate::NO_ROOT.to_owned()));
            };
            let item = catalog.get(&key(path)).ok_or_else(|| {
                LibraryRefusal::new(
                    RefusalClass::NotFound,
                    NOT_FOUND,
                    json!({ "path": path.as_str() }),
                )
            })?;
            let registry = &self.values.registry;
            let (reaching, agents_direct) = referrers_of(catalog, registry, &item.path);
            let lists_it = |preset: &LibPath| match catalog.get(&key(preset)).map(|p| &p.content) {
                Some(ItemContent::Preset { doc }) => {
                    preset_refs(doc).any(|target| key(target) == key(&item.path))
                }
                _ => false,
            };
            let (direct, indirect): (Vec<&LibPath>, Vec<&LibPath>) =
                reaching.iter().partition(|preset| lists_it(preset));
            let mut agents_indirect: Vec<AgentId> = registry
                .attachments()
                .into_iter()
                .filter(|(id, attached)| {
                    !agents_direct.contains(id)
                        && attached.presets.iter().any(|preset| {
                            reaching.iter().any(|reached| {
                                same_path(Path::new(&preset.path), Path::new(reached.as_str()))
                            })
                        })
                })
                .map(|(id, _)| id)
                .collect();
            agents_indirect.sort_unstable();
            let names = |paths: Vec<&LibPath>| -> Vec<String> {
                paths.into_iter().map(|p| p.as_str().to_owned()).collect()
            };
            let (excerpt, settings_fields) = match &item.content {
                ItemContent::Instruction { excerpt } => (Some(excerpt.clone()), Vec::new()),
                ItemContent::Settings { doc, .. } => (
                    None,
                    doc.raw.iter().map(|field| field.name.clone()).collect(),
                ),
                _ => (None, Vec::new()),
            };
            Ok(ReadAnswer {
                row: row(item, reaching.len() + agents_direct.len()),
                absolute: item.path.to_path(root),
                presets_direct: names(direct),
                presets_indirect: names(indirect),
                agents_direct,
                agents_indirect,
                excerpt,
                settings_fields,
            })
        })
    }

    /// Walks and reads `range` under `root` outside every lock.
    fn build_scan(
        &self,
        root: &Path,
        range: Option<&LibPath>,
        tags: &Tags,
    ) -> io::Result<ScanResult> {
        let read = |path: &Path| self.store.read(path, LIBRARY_READ_CAP);
        self.build_scan_reading(root, range, tags, &read)
    }

    /// [`LibraryService::build_scan`] with the file reads going through `read` (the launch overlay keeps the bytes).
    fn build_scan_reading(
        &self,
        root: &Path,
        range: Option<&LibPath>,
        tags: &Tags,
        read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
    ) -> io::Result<ScanResult> {
        // The racy rule's reference — taken before the walk (`ScanResult::build`).
        let now = SystemTime::now();
        let listing = self.store.scan(root, range, &|| self.gate.next_seq())?;
        Ok(ScanResult::build(
            listing,
            range,
            tags,
            read,
            &*self.values.backends,
            now,
        ))
    }

    /// One path read through the same partial scan a refresh of it would run — so it admits exactly what the scan
    /// admits (names, links, kinds), and every read draws its number first.
    fn observe(&self, root: &Path, path: &LibPath) -> Observed {
        match self.build_scan(root, Some(path), &Tags::new()) {
            Err(e) => {
                tracing::debug!(
                    path = path.as_str(),
                    "build_scan 실패, Observed::Refused로 읽지 않음: {}",
                    escape_controls(&e.to_string())
                );
                Observed::Refused
            }
            Ok(result)
                if result
                    .skipped_links
                    .iter()
                    .any(|link| key(link) == key(path)) =>
            {
                Observed::ThroughLink
            }
            Ok(result) => Observed::Items(
                result
                    .entries
                    .into_iter()
                    .filter_map(|entry| match entry {
                        ScanItem::Fresh(item) => Some(item),
                        ScanItem::Unchanged(..) => None,
                    })
                    .collect(),
            ),
        }
    }

    fn answer(&self, state: &State, folder: Option<&LibPath>) -> ListAnswer {
        let rows = state
            .catalog
            .children(folder)
            .into_iter()
            .map(|item| {
                let references = reference_count(&state.catalog, &self.values.registry, &item.path);
                row(item, references)
            })
            .collect();
        let want = folder.map(key);
        let mut subfolders: Vec<&LibPath> = state
            .catalog
            .folders()
            .filter(|sub| sub.parent().as_ref().map(key) == want)
            .filter(|sub| state.catalog.get(&key(sub)).is_none())
            .collect();
        subfolders.sort();
        let folders = subfolders
            .into_iter()
            .map(|sub| FolderRow {
                path: sub.as_str().to_owned(),
                name: sub.file_name().to_owned(),
            })
            .collect();
        ListAnswer {
            folder: folder.cloned(),
            rows,
            folders,
            builtins: Vec::new(),
        }
    }
}

/// The mode of an agent command (`command_field`). A command without one is `Terminal`, as the profile reads an
/// absent `output_format`.
fn output_format(command: &AgentCommand) -> AgentOutputFormat {
    command_field(command, "output_format").unwrap_or_default()
}

/// The pass-through arguments of an agent command (`command_field`); none for a command without them.
fn extra_args(command: &AgentCommand) -> Vec<String> {
    command_field(command, "extra_args").unwrap_or_default()
}

/// A field of an agent command, read through the command's serde shape the way `agent::launch::backend_word` reads
/// the backend word, so no backend variant is named here (TRD S25 step-7 §1-1). `None` = the command has no such
/// field.
fn command_field<T: serde::de::DeserializeOwned>(command: &AgentCommand, name: &str) -> Option<T> {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value.get(name).cloned())
        .and_then(|field| serde_json::from_value(field).ok())
}

fn merge_from(gate: &Gate, noted: Numbers, slot: &Mutex<Option<Vec<Item>>>) {
    let work = lock(slot).take();
    if let Some(items) = work {
        gate.late_merge(noted, |catalog, may_insert| {
            catalog.apply_items(items, may_insert)
        });
    }
}

fn is_known(catalog: &Catalog, path: &LibPath) -> bool {
    let want = key(path);
    catalog.get(&want).is_some() || catalog.folders().any(|folder| key(folder) == want)
}

fn root_unusable(root: &Path, why: String) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Unavailable,
        LIBRARY_ROOT_UNUSABLE,
        json!({ "path": root.display().to_string(), "why": why }),
    )
}

fn failed(why: String) -> RefreshReport {
    RefreshReport {
        errors: vec![(None, why)],
        ..RefreshReport::default()
    }
}

fn row(item: &Item, references: usize) -> ListRow {
    let file_name = || item.path.file_name().to_owned();
    let (name, description, state) = match &item.content {
        ItemContent::Instruction { excerpt } => (file_name(), excerpt.clone(), None),
        ItemContent::Skill {
            name, description, ..
        } => (name.clone(), description.clone(), None),
        ItemContent::SkillFile => (file_name(), String::new(), None),
        ItemContent::Preset { doc } => (
            file_name(),
            doc.description.clone().unwrap_or_default(),
            None,
        ),
        // TRD S25 step-7 §3-6: a refusal in any backend's field shows on the asset's own row.
        ItemContent::Settings { doc, verdict } => (
            file_name(),
            doc.description.clone().unwrap_or_default(),
            verdict.problems.first().map(|(_, refusal)| refusal.clone()),
        ),
        ItemContent::Broken { reason } => (file_name(), String::new(), Some(reason.clone())),
    };
    ListRow {
        path: item.path.as_str().to_owned(),
        kind: item.kind.clone(),
        name,
        state,
        references,
        description,
    }
}

fn through_link_row(path: &LibPath) -> ListRow {
    ListRow {
        path: path.as_str().to_owned(),
        kind: ItemKind::Broken { claimed: None },
        name: path.file_name().to_owned(),
        state: Some(Reason {
            code: PATH_THROUGH_LINK.to_owned(),
            args: json!({ "path": path.as_str() }),
            line: None,
        }),
        references: 0,
        description: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::Condvar;
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use engram_dashboard_agent::profile::{AgentProfile, AttachedPreset, ProfileStore, StoreError};
    use engram_dashboard_base::sync::wait_timeout;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::excerpt::excerpt;
    use crate::folder::{
        FolderListing, FolderOps, LaunchWaits, Spawn, ThreadSpawn, IO_LIMIT, LIST_CAP,
    };
    use crate::path::BuiltinId;
    use crate::store::mem::{ListFailure, MemStore, OLD};
    use crate::store::{ContentTag, ScanListing, TrashBundle};

    fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    // ── the store: MemStores under fixed roots, a log of scans, one hold point a test arms ──────────────────

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum At {
        /// At the walk's first sequence draw — the walk has started and listed nothing yet.
        Start,
        /// After the listing, before it is returned — before any file is read and before the apply.
        End,
    }

    #[derive(Default)]
    struct Hold {
        armed: Option<(At, PathBuf)>,
        parked: bool,
        released: bool,
    }

    struct TestStore {
        roots: Vec<(PathBuf, MemStore)>,
        scans: Mutex<Vec<(PathBuf, Option<LibPath>)>>,
        read_caps: Mutex<Vec<u64>>,
        /// The next scan parks at `Start` (if armed) and then fails as an unreadable root.
        fail_next_scan: Mutex<bool>,
        hold: Mutex<Hold>,
        moved: Condvar,
    }

    impl TestStore {
        fn new(roots: &[&str]) -> Arc<Self> {
            Arc::new(TestStore {
                roots: roots
                    .iter()
                    .map(|root| (PathBuf::from(root), MemStore::new(Path::new(root))))
                    .collect(),
                scans: Mutex::default(),
                read_caps: Mutex::default(),
                fail_next_scan: Mutex::default(),
                hold: Mutex::default(),
                moved: Condvar::new(),
            })
        }

        fn mem(&self, root: &str) -> &MemStore {
            self.at_root(Path::new(root))
        }

        fn at_root(&self, root: &Path) -> &MemStore {
            &self
                .roots
                .iter()
                .find(|(at, _)| at == root)
                .expect("a root of the test store")
                .1
        }

        /// The store of the deepest root holding `path` — nested roots model one disk only while their files agree.
        fn holding(&self, path: &Path) -> &MemStore {
            &self
                .roots
                .iter()
                .filter(|(root, _)| path.starts_with(root))
                .max_by_key(|(root, _)| root.components().count())
                .expect("a path under a root of the test store")
                .1
        }

        fn scans(&self) -> Vec<(String, Option<String>)> {
            lock(&self.scans)
                .iter()
                .map(|(root, under)| {
                    let under = under.as_ref().map(|under| under.as_str().to_owned());
                    (root.display().to_string(), under)
                })
                .collect()
        }

        fn hold_at(&self, at: At, root: &str) {
            lock(&self.hold).armed = Some((at, PathBuf::from(root)));
        }

        fn wait_held(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut hold = lock(&self.hold);
            while !hold.parked {
                let now = Instant::now();
                assert!(now < deadline, "no scan reached the hold");
                hold = wait_timeout(&self.moved, hold, deadline - now).0;
            }
        }

        fn release(&self) {
            lock(&self.hold).released = true;
            self.moved.notify_all();
        }

        fn park(&self, at: At, root: &Path) {
            let mut hold = lock(&self.hold);
            if hold.armed.as_ref() != Some(&(at, root.to_path_buf())) {
                return;
            }
            hold.armed = None;
            hold.parked = true;
            self.moved.notify_all();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !hold.released {
                let now = Instant::now();
                assert!(now < deadline, "a held scan was never released");
                hold = wait_timeout(&self.moved, hold, deadline - now).0;
            }
            hold.released = false;
            hold.parked = false;
        }
    }

    fn scan(root: &str, under: Option<&str>) -> (String, Option<String>) {
        (root.to_owned(), under.map(str::to_owned))
    }

    impl LibraryStore for TestStore {
        fn scan(
            &self,
            root: &Path,
            under: Option<&LibPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            lock(&self.scans).push((root.to_path_buf(), under.cloned()));
            if std::mem::take(&mut *lock(&self.fail_next_scan)) {
                self.park(At::Start, root);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let started = Cell::new(false);
            let listing = self.at_root(root).scan(root, under, &|| {
                if !started.replace(true) {
                    self.park(At::Start, root);
                }
                seq()
            });
            self.park(At::End, root);
            listing
        }

        fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
            self.at_root(root).first_link(root, rel)
        }

        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            lock(&self.read_caps).push(cap);
            self.holding(path).read(path, cap)
        }

        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            self.holding(path).tag(path)
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.holding(path).write_atomic(path, bytes)
        }

        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.holding(path).create_dir(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.holding(from).rename(from, to)
        }

        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.holding(from).copy(from, to)
        }

        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            self.at_root(root).trash(root, path, bundle)
        }

        fn exists(&self, path: &Path) -> bool {
            self.holding(path).exists(path)
        }
    }

    // ── folder ops, spawners, the registry ───────────────────────────────────────────────────────────────

    /// Folder ops whose stat answers from a table (default `Ok`); nothing else is modeled.
    #[derive(Default)]
    struct StatOps(Mutex<HashMap<PathBuf, FolderStatus>>);

    impl FolderOps for StatOps {
        fn stat(&self, p: &Path) -> FolderStatus {
            lock(&self.0).get(p).cloned().unwrap_or(FolderStatus::Ok)
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::Error::other("not modeled"))
        }

        fn read(&self, _: &Path, _: u64) -> io::Result<String> {
            Err(io::Error::other("not modeled"))
        }
    }

    /// Spawns threads and keeps their handles, so a test can wait for every job.
    #[derive(Default)]
    struct JoinSpawn(Mutex<Vec<JoinHandle<()>>>);

    impl Spawn for JoinSpawn {
        fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let handle = thread::Builder::new().name(name.to_owned()).spawn(job)?;
            lock(&self.0).push(handle);
            Ok(())
        }
    }

    impl JoinSpawn {
        fn join_all(&self) {
            for handle in std::mem::take(&mut *lock(&self.0)) {
                join(handle);
            }
        }
    }

    struct FailingSpawn;

    impl Spawn for FailingSpawn {
        fn spawn(&self, _: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            drop(job);
            Err(io::Error::other("no threads left"))
        }
    }

    struct Fixed(Vec<AgentProfile>);

    impl ProfileStore for Fixed {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    fn agent(presets: &[&str]) -> AgentProfile {
        let command = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let mut profile = AgentProfile::new(
            "a".to_owned(),
            command,
            PathBuf::from("/work"),
            Vec::new(),
            false,
        );
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    /// The root generation the test services start at — not 0, so an absolute value cannot pass by chance.
    const ROOT_BASE: u64 = 1_000;

    fn service(
        root: Option<&str>,
        store: &Arc<TestStore>,
        profiles: Vec<AgentProfile>,
        ops: Arc<dyn FolderOps>,
        spawn: Arc<dyn Spawn>,
    ) -> Arc<LibraryService> {
        let io = BoundedIo::new(IO_LIMIT, LaunchWaits::LAUNCH, LIST_CAP, ops, spawn);
        service_io(root, store, profiles, None, io)
    }

    fn service_io(
        root: Option<&str>,
        store: &Arc<TestStore>,
        profiles: Vec<AgentProfile>,
        builtin_dir: Option<&Path>,
        io: BoundedIo,
    ) -> Arc<LibraryService> {
        let state = Path::new("/state");
        let config = match root {
            Some(root) => LibraryConfig::usable(state, RootSetting::Explicit(PathBuf::from(root))),
            None => LibraryConfig::missing(state),
        };
        let values = LibraryValues {
            registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles)))),
            backends: Arc::new(AgentBackends),
            config,
            default_root: PathBuf::from("/default-library"),
            daemon_id: "test-daemon".to_owned(),
            root_generation_base: ROOT_BASE,
            builtin_dir: builtin_dir.map(Path::to_path_buf),
            launch_root: PathBuf::from("/launch"),
            home: None,
            env: Arc::new(|_| None),
        };
        let store: Arc<dyn LibraryStore> = Arc::clone(store) as Arc<dyn LibraryStore>;
        Arc::new(LibraryService::new(values, store, io))
    }

    fn service_on(roots: &[&str], root: Option<&str>) -> (Arc<LibraryService>, Arc<TestStore>) {
        let store = TestStore::new(roots);
        let svc = service(
            root,
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        (svc, store)
    }

    /// A service on `/lib` with no agents, every stat `Ok` and threads for jobs.
    fn library() -> (Arc<LibraryService>, Arc<TestStore>) {
        service_on(&["/lib"], Some("/lib"))
    }

    /// Joins `handle`, failing after 20 s instead of hanging.
    fn join<T>(handle: JoinHandle<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "a test thread hung");
            thread::sleep(Duration::from_millis(2));
        }
        handle.join().expect("the test thread panicked")
    }

    fn on_thread<T: Send + 'static>(
        svc: &Arc<LibraryService>,
        f: impl FnOnce(&LibraryService) -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let svc = Arc::clone(svc);
        thread::spawn(move || f(&svc))
    }

    /// What a launch check's overlay read hands its merge: the numbers noted first, then the items read.
    fn check_read(svc: &LibraryService, rel: &str) -> (Numbers, Vec<Item>) {
        let noted = svc.numbers();
        match svc.observe(Path::new("/lib"), &lp(rel)) {
            Observed::Items(items) => (noted, items),
            Observed::ThroughLink | Observed::Refused => panic!("{rel} is not readable"),
        }
    }

    fn merge(svc: &LibraryService, (noted, items): (Numbers, Vec<Item>)) {
        svc.late_merge(noted, |catalog, may_insert| {
            catalog.apply_items(items, may_insert)
        });
    }

    fn has(svc: &LibraryService, rel: &str) -> bool {
        svc.with_read(|state| state.catalog.get(&key(&lp(rel))).is_some())
    }

    /// The version of the file the catalog's item was read from (`MemStore::file_version`).
    fn version(svc: &LibraryService, rel: &str) -> u64 {
        svc.with_read(|state| {
            let item = state.catalog.get(&key(&lp(rel))).expect(rel);
            item.tag.modified.duration_since(OLD).unwrap().as_secs()
        })
    }

    fn root_of(svc: &LibraryService) -> Option<PathBuf> {
        svc.with_read(|state| state.root.clone())
    }

    fn paths(answer: &ListAnswer) -> Vec<&str> {
        answer.rows.iter().map(|row| row.path.as_str()).collect()
    }

    fn folder_paths(answer: &ListAnswer) -> Vec<&str> {
        answer
            .folders
            .iter()
            .map(|folder| folder.path.as_str())
            .collect()
    }

    // ── list ─────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn list_rows_carry_kind_name_state_description_and_reference_counts_and_never_rescan() {
        let store = TestStore::new(&["/lib"]);
        let svc = service(
            Some("/lib"),
            &store,
            vec![agent(&["top.yaml"])],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let mem = store.mem("/lib");
        let rules = "# Rules\n\nKeep it short.\n";
        mem.file("rules/a.md", rules);
        mem.file(
            "top.yaml",
            "kind: preset\nversion: 1\ndescription: the top\ninstructions:\n  - rules/a.md\nskills:\n  - skills/s\n",
        );
        mem.file(
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: does s\n---\nbody\n",
        );
        mem.file("broken.yaml", "kind: preset\nversion: [\n");
        mem.file("plain.yaml", "a: 1\n");

        let root = svc.list(None).unwrap();
        assert_eq!(paths(&root), ["broken.yaml", "top.yaml"]);
        assert_eq!(folder_paths(&root), ["rules", "skills"]);
        let broken = &root.rows[0];
        assert!(matches!(broken.kind, ItemKind::Broken { .. }));
        assert_eq!(broken.state.as_ref().unwrap().code, "yaml-syntax");
        let top = &root.rows[1];
        assert_eq!(
            (
                top.kind.clone(),
                top.name.as_str(),
                top.description.as_str()
            ),
            (ItemKind::Preset, "top.yaml", "the top")
        );
        assert_eq!((top.state.clone(), top.references), (None, 1), "the agent");

        let rules_rows = svc.list(Some(&lp("rules"))).unwrap();
        assert_eq!(paths(&rules_rows), ["rules/a.md"]);
        let a = &rules_rows.rows[0];
        assert_eq!(a.kind, ItemKind::Instruction);
        assert_eq!(a.description, excerpt(rules));
        assert_eq!(a.references, 1, "the preset");

        let skills = svc.list(Some(&lp("skills"))).unwrap();
        assert_eq!(paths(&skills), ["skills/s"]);
        assert!(
            skills.folders.is_empty(),
            "a skill folder is an item, not a folder line"
        );
        let s = &skills.rows[0];
        assert_eq!(
            (
                s.kind.clone(),
                s.name.as_str(),
                s.description.as_str(),
                s.references
            ),
            (ItemKind::Skill, "s", "does s", 1)
        );
        assert!(root.builtins.is_empty());
        assert_eq!(store.scans(), [scan("/lib", None)], "only the first scan");
    }

    #[test]
    fn list_answers_the_plain_subfolders_directly_in_the_folder() {
        let (svc, store) = library();
        store.mem("/lib").file("a/b/c.md", "c");
        store.mem("/lib").file("a/z/d.md", "d");
        let root = svc.list(None).unwrap();
        assert!(root.rows.is_empty());
        assert_eq!(
            root.folders,
            [FolderRow {
                path: "a".to_owned(),
                name: "a".to_owned()
            }]
        );
        let a = svc.list(Some(&lp("a"))).unwrap();
        assert!(a.rows.is_empty());
        assert_eq!(
            folder_paths(&a),
            ["a/b", "a/z"],
            "direct children only, by path"
        );
        assert_eq!(a.folders[1].name, "z");
    }

    #[test]
    fn every_catalog_entrance_passes_the_whole_first_scan_first() {
        for entrance in 0..4 {
            let (svc, store) = library();
            store.mem("/lib").file("a/x.md", "x");
            store.mem("/lib").file("b/y.md", "y");
            match entrance {
                0 => drop(svc.list(None).unwrap()),
                1 => drop(svc.list(Some(&lp("a"))).unwrap()),
                2 => drop(svc.refresh(Some(&lp("a")))),
                _ => svc.first_scan_join().unwrap(),
            }
            assert_eq!(store.scans(), [scan("/lib", None)], "entrance {entrance}");
            assert!(has(&svc, "b/y.md"), "entrance {entrance}");
        }
    }

    #[test]
    fn readers_attach_to_a_walking_first_scan() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None).unwrap());
        store.wait_held();
        let second = on_thread(&svc, |svc| svc.list(None).unwrap());
        svc.gate
            .wait_for("the second reader waits", |_, waiting| waiting == 1);
        store.release();
        let (first, second) = (join(first), join(second));
        assert_eq!(paths(&first), ["a.md"]);
        assert_eq!(first, second);
        assert_eq!(store.scans(), [scan("/lib", None)]);
    }

    #[test]
    fn the_safety_net_merges_one_path_the_catalog_lacks_and_refuses_links_dot_and_temp_names() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file("a.md", "a");
        mem.link("lnk");
        mem.file(".hidden.md", "h");
        mem.file("x.md.tmp12.0", "t");
        svc.list(None).unwrap();
        mem.file("later.md", "l");
        assert!(
            svc.list(Some(&lp("later.md"))).unwrap().rows.is_empty(),
            "a file has no children"
        );
        assert!(has(&svc, "later.md"), "merged by the safety net");

        let through = svc.list(Some(&lp("lnk/x.md"))).unwrap();
        assert_eq!(paths(&through), ["lnk/x.md"]);
        assert_eq!(
            through.rows[0].state.as_ref().unwrap().code,
            PATH_THROUGH_LINK
        );
        assert!(!has(&svc, "lnk/x.md"));
        for name in [".hidden.md", "x.md.tmp12.0"] {
            assert!(svc.list(Some(&lp(name))).unwrap().rows.is_empty(), "{name}");
            assert!(!has(&svc, name), "{name}");
        }
        // The same admission gates every late merge's read.
        let root = Path::new("/lib");
        assert!(matches!(
            svc.observe(root, &lp("lnk/x.md")),
            Observed::ThroughLink
        ));
        assert!(matches!(
            svc.observe(root, &lp(".hidden.md")),
            Observed::Refused
        ));
        assert!(matches!(
            svc.observe(root, &lp("x.md.tmp12.0")),
            Observed::Refused
        ));
    }

    // ── refresh ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_folder_refresh_before_the_first_scan_answers_its_share_of_a_whole_scan() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - b/x.md\n  - a/missing.md\n",
        );
        mem.file("b/x.md", "x");
        mem.file("b/broken.yaml", "kind: preset\nversion: [\n");
        let report = svc.refresh(Some(&lp("a")));
        assert_eq!(store.scans(), [scan("/lib", None)]);
        let missing = Reason {
            code: NOT_FOUND.to_owned(),
            args: json!({ "path": "a/missing.md" }),
            line: None,
        };
        // `b/x.md` is found (the whole library was scanned) and `b/broken.yaml` is outside `a`.
        assert_eq!(report.problems, [(lp("a/p.yaml"), missing)]);
        assert_eq!(report.changed, 1);
    }

    #[test]
    fn a_folder_refresh_arriving_at_a_walking_first_scan_scans_its_folder_after_it() {
        let (svc, store) = library();
        store.mem("/lib").file("a/x.md", "x");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None).unwrap());
        store.wait_held();
        let refresh = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the refresh waits for the first scan", |_, waiting| {
                waiting == 1
            });
        store.release();
        join(first);
        join(refresh);
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", Some("a"))]);
    }

    #[test]
    fn a_folder_refresh_whose_waited_first_scan_failed_goes_through_a_whole_one() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - b/x.md\n",
        );
        store.mem("/lib").file("b/x.md", "x");
        store.mem("/lib").file("c.md", "c");
        *lock(&store.fail_next_scan) = true;
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(None));
        store.wait_held();
        let refresh = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the refresh waits for the first scan", |_, waiting| {
                waiting == 1
            });
        store.release();
        let failed = join(first).expect_err("the failed first scan is the list's answer");
        assert_eq!(failed.code, LIBRARY_UNAVAILABLE);
        let report = join(refresh);
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", None)]);
        assert!(report.problems.is_empty(), "no false not-found: {report:?}");
        assert!(report.errors.is_empty());
        assert_eq!(report.changed, 1);
        assert_eq!(
            paths(&svc.list(None).unwrap()),
            ["c.md"],
            "the second scan was applied"
        );
    }

    #[test]
    fn a_refresh_arriving_at_a_walking_covering_scan_scans_again_and_answers_the_new_bytes() {
        let (svc, store) = library();
        store.mem("/lib").file_version("a/x.md", "old", 1);
        svc.list(None).unwrap();
        store.hold_at(At::End, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("a/x.md", "new", 2);
        let late = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        svc.gate
            .wait_for("the late refresh has its own ticket", |line, _| {
                line == [("scan", true), ("scan", false)]
            });
        store.release();
        assert_eq!(join(walking).changed, 0);
        assert_eq!(join(late).changed, 1);
        assert_eq!(version(&svc, "a/x.md"), 2);
        assert_eq!(
            store.scans(),
            [
                scan("/lib", None),
                scan("/lib", None),
                scan("/lib", Some("a"))
            ]
        );
    }

    #[test]
    fn refresh_reports_skipped_links_walk_errors_and_no_kind_and_reads_with_the_library_cap() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file("c.md", "c");
        mem.file("sub/x.md", "x");
        mem.file("plain.yaml", "a: 1\n");
        mem.link("lnk");
        mem.fail_listing("sub", ListFailure::Open);
        let report = svc.refresh(None);
        assert_eq!(report.skipped_links, [lp("lnk")]);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert_eq!(report.errors[0].0, Some(lp("sub")));
        assert_eq!(report.no_kind, [lp("plain.yaml")]);
        let caps = lock(&store.read_caps).clone();
        assert!(!caps.is_empty());
        assert!(caps.iter().all(|&cap| cap == LIBRARY_READ_CAP), "{caps:?}");
        assert_eq!(LIBRARY_READ_CAP, 16 * 1024 * 1024);
    }

    #[test]
    fn without_a_root_a_refresh_and_a_list_answer_why() {
        let (svc, store) = service_on(&["/lib"], None);
        assert_eq!(svc.refresh(None).errors, [(None, gate::NO_ROOT.to_owned())]);
        let unavailable = Reason {
            code: LIBRARY_UNAVAILABLE.to_owned(),
            args: json!({ "message": gate::NO_ROOT }),
            line: None,
        };
        assert_eq!(svc.list(None), Err(unavailable));
        assert!(store.scans().is_empty());
    }

    // ── late merges and scans landing in either order ────────────────────────────────────────────────────

    #[test]
    fn a_file_read_by_a_check_and_then_deleted_is_gone_after_the_scan_that_listed_its_folder_in_either_order(
    ) {
        for merge_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file("d/x.md", "x");
            store.mem("/lib").file("d/y.md", "y");
            svc.list(None).unwrap();
            let read = check_read(&svc, "d/x.md");
            store.mem("/lib").remove("d/x.md");
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("d"))));
            store.wait_held();
            if merge_first {
                merge(&svc, read);
                store.release();
                join(scan);
            } else {
                store.release();
                join(scan);
                merge(&svc, read);
            }
            assert!(!has(&svc, "d/x.md"), "merge first: {merge_first}");
            assert!(has(&svc, "d/y.md"));
        }
    }

    #[test]
    fn a_file_created_after_the_scan_listed_its_folder_lands_per_order_and_the_safety_net_inserts_it_later(
    ) {
        for merge_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file("d/y.md", "y");
            svc.list(None).unwrap();
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("d"))));
            store.wait_held();
            store.mem("/lib").file("d/x.md", "x");
            let read = check_read(&svc, "d/x.md");
            if merge_first {
                merge(&svc, read);
                store.release();
                join(scan);
                assert!(has(&svc, "d/x.md"), "read after the listing: kept");
            } else {
                store.release();
                join(scan);
                merge(&svc, read);
                assert!(
                    !has(&svc, "d/x.md"),
                    "a swap-in landed between: the insert is dropped"
                );
                svc.list(Some(&lp("d/x.md"))).unwrap();
                assert!(
                    has(&svc, "d/x.md"),
                    "the safety net with no swap-in between"
                );
            }
        }
    }

    #[test]
    fn a_launch_merge_of_folder_b_survives_the_apply_of_a_folder_a_walk() {
        let (svc, store) = library();
        store.mem("/lib").file_version("a/x.md", "x", 1);
        store.mem("/lib").file_version("b/y.md", "y", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("b/y.md", "y2", 2);
        let read = check_read(&svc, "b/y.md");
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("a"))));
        store.wait_held();
        merge(&svc, read);
        store.release();
        join(scan);
        assert_eq!(version(&svc, "b/y.md"), 2);
    }

    #[test]
    fn a_merge_inside_the_range_read_later_survives_the_whole_scan_apply() {
        let (svc, store) = library();
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        // The scan lists x at version 2 and reads it only after the hold — by then it holds version 3.
        store.mem("/lib").file_version("x.md", "v2", 2);
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("x.md", "v3", 3);
        merge(&svc, check_read(&svc, "x.md"));
        store.release();
        join(scan);
        assert_eq!(version(&svc, "x.md"), 3, "the later read stays");
    }

    #[test]
    fn a_safety_net_insert_landing_during_a_walk_survives_its_apply() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        svc.list(None).unwrap();
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file("n.md", "n");
        svc.list(Some(&lp("n.md"))).unwrap();
        assert!(has(&svc, "n.md"));
        store.release();
        join(scan);
        assert!(has(&svc, "n.md"), "read after the listing");
    }

    #[test]
    fn with_the_numbers_unchanged_the_later_read_wins_in_either_landing_order() {
        for later_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file_version("x.md", "v1", 1);
            svc.list(None).unwrap();
            store.mem("/lib").file_version("x.md", "v2", 2);
            let older = check_read(&svc, "x.md");
            store.mem("/lib").file_version("x.md", "v3", 3);
            let newer = check_read(&svc, "x.md");
            if later_first {
                merge(&svc, newer);
                merge(&svc, older);
            } else {
                merge(&svc, older);
                merge(&svc, newer);
            }
            assert_eq!(version(&svc, "x.md"), 3, "later first: {later_first}");
        }
    }

    #[test]
    fn a_merge_read_before_a_change_does_not_revive_what_the_change_removed() {
        let (svc, store) = library();
        store.mem("/lib").file("p.md", "p");
        svc.list(None).unwrap();
        let read = check_read(&svc, "p.md");
        svc.change(|catalog, _, seq| {
            store.mem("/lib").remove("p.md");
            let k = key(&lp("p.md"));
            let removed = catalog.remove(&k, seq());
            ((), if removed { vec![k] } else { Vec::new() })
        });
        merge(&svc, read);
        assert!(!has(&svc, "p.md"));
    }

    #[test]
    fn a_handed_off_merge_waits_behind_a_held_write_lock_and_lands_after_it() {
        let store = TestStore::new(&["/lib"]);
        let spawn = Arc::new(JoinSpawn::default());
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "x.md");
        {
            let held = svc.gate.lib().write().unwrap();
            // The hand-off returns while the lock is held (merging in place, it would hang here).
            join(on_thread(&svc, move |svc| svc.hand_off_merge(noted, items)));
            let still = held.catalog.get(&key(&lp("x.md"))).unwrap().tag;
            assert_eq!(
                still.modified,
                OLD + Duration::from_secs(1),
                "not merged yet"
            );
        }
        spawn.join_all();
        assert_eq!(version(&svc, "x.md"), 2, "delayed, not dropped");
    }

    #[test]
    fn a_hand_off_whose_job_cannot_start_merges_in_place() {
        let store = TestStore::new(&["/lib"]);
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::new(FailingSpawn),
        );
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(None).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "x.md");
        svc.hand_off_merge(noted, items);
        assert_eq!(version(&svc, "x.md"), 2);
    }

    // ── changes ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_change_drawn_during_a_walk_waits_for_its_apply_and_is_not_undone() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(None).unwrap();
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        let disk = Arc::clone(&store);
        let change = on_thread(&svc, move |svc| {
            svc.change(|catalog, _, seq| {
                disk.mem("/lib").remove("x.md");
                let k = key(&lp("x.md"));
                let removed = catalog.remove(&k, seq());
                (removed, vec![k])
            })
        });
        svc.gate
            .wait_for("the change waits behind the walk", |line, _| {
                line == [("scan", true), ("change", false)]
            });
        store.release();
        join(scan);
        assert!(join(change), "the change removed what the scan had applied");
        assert!(!has(&svc, "x.md"));
    }

    #[test]
    fn a_change_generation_moved_during_a_walk_makes_the_scan_walk_again() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(None).unwrap();
        store.hold_at(At::Start, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        svc.gate.change_unordered(|catalog| {
            store.mem("/lib").remove("x.md");
            catalog.remove(&key(&lp("x.md")), svc.gate.next_seq());
        });
        store.release();
        join(walking);
        assert!(!has(&svc, "x.md"));
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/lib", None), scan("/lib", None)]
        );
    }

    // ── root change ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn readers_see_the_old_root_during_the_new_roots_walk() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        store.mem("/lib2").file("b.md", "b");
        assert_eq!(paths(&svc.list(None).unwrap()), ["a.md"]);
        store.hold_at(At::Start, "/lib2");
        let change = on_thread(&svc, |svc| svc.change_root(PathBuf::from("/lib2")));
        store.wait_held();
        assert_eq!(
            paths(&svc.list(None).unwrap()),
            ["a.md"],
            "never an empty catalog"
        );
        store.release();
        join(change).unwrap();
        assert_eq!(paths(&svc.list(None).unwrap()), ["b.md"]);
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib2")));
    }

    #[test]
    fn a_nested_root_change_keeps_a_file_whose_tag_equals_an_old_item_of_the_same_key() {
        let (svc, store) = service_on(&["/lib", "/lib/team"], Some("/lib"));
        // `X.md` under the old root and `team/X.md` (the new root's `X.md`) have equal tags: same size, same time.
        store.mem("/lib").file_version("X.md", "top!", 1);
        store.mem("/lib").file_version("team/X.md", "team", 1);
        store.mem("/lib/team").file_version("X.md", "team", 1);
        assert_eq!(svc.list(None).unwrap().rows[0].description, excerpt("top!"));
        svc.change_root(PathBuf::from("/lib/team")).unwrap();
        let rows = svc.list(None).unwrap();
        assert_eq!(paths(&rows), ["X.md"]);
        assert_eq!(rows.rows[0].description, excerpt("team"));
    }

    #[test]
    fn a_first_start_root_change_raises_the_first_scan_for_the_readers_waiting_on_it() {
        let (svc, store) = service_on(&["/lib"], None);
        store.mem("/lib").file("a.md", "a");
        store.hold_at(At::Start, "/lib");
        let change = on_thread(&svc, |svc| svc.change_root(PathBuf::from("/lib")));
        store.wait_held();
        let reader = on_thread(&svc, |svc| svc.list(None).unwrap());
        svc.gate
            .wait_for("the reader waits for the root walk", |_, waiting| {
                waiting == 1
            });
        store.release();
        join(change).unwrap();
        assert_eq!(paths(&join(reader)), ["a.md"]);
        assert_eq!(store.scans(), [scan("/lib", None)], "no second scan");
    }

    #[test]
    fn a_root_change_whose_walk_fails_at_the_root_publishes_nothing() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        svc.list(None).unwrap();
        store.mem("/lib2").fail_listing("", ListFailure::Open);
        assert!(svc.change_root(PathBuf::from("/lib2")).is_err());
        let answer = join(on_thread(&svc, |svc| svc.list(None).unwrap()));
        assert_eq!(paths(&answer), ["a.md"]);
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib")));
        let report = join(on_thread(&svc, |svc| svc.refresh(None)));
        assert!(report.errors.is_empty(), "the ticket was released");
    }

    #[test]
    fn a_new_root_that_does_not_stat_as_a_folder_is_refused_before_any_walk() {
        let store = TestStore::new(&["/lib", "/lib2"]);
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(PathBuf::from("/lib2"), FolderStatus::Missing);
        let svc = service(Some("/lib"), &store, Vec::new(), ops, Arc::new(ThreadSpawn));
        assert!(svc.change_root(PathBuf::from("/lib2")).is_err());
        assert!(store.scans().is_empty());
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib")));
    }

    // ── resolution entrances ─────────────────────────────────────────────────────────────────────────────

    /// Folder ops that count every stat by path, answer from a table (default `Ok`), and hold the stat of a held path
    /// until the test releases it.
    #[derive(Default)]
    struct CountOps {
        statuses: HashMap<PathBuf, FolderStatus>,
        held: Vec<PathBuf>,
        stats: Mutex<HashMap<PathBuf, usize>>,
        released: Mutex<bool>,
        moved: Condvar,
    }

    impl CountOps {
        fn stats_of(&self, path: &Path) -> usize {
            lock(&self.stats).get(path).copied().unwrap_or(0)
        }

        fn release(&self) {
            *lock(&self.released) = true;
            self.moved.notify_all();
        }
    }

    impl FolderOps for CountOps {
        fn stat(&self, p: &Path) -> FolderStatus {
            *lock(&self.stats).entry(p.to_path_buf()).or_default() += 1;
            if self.held.iter().any(|held| held == p) {
                let deadline = Instant::now() + Duration::from_secs(20);
                let mut released = lock(&self.released);
                while !*released && Instant::now() < deadline {
                    released = wait_timeout(&self.moved, released, deadline - Instant::now()).0;
                }
            }
            self.statuses.get(p).cloned().unwrap_or(FolderStatus::Ok)
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::Error::other("not modeled"))
        }

        fn read(&self, _: &Path, _: u64) -> io::Result<String> {
            Err(io::Error::other("not modeled"))
        }
    }

    fn coded(command: AgentCommand, presets: &[&str]) -> AgentProfile {
        let mut profile = agent(presets);
        profile.command = command;
        profile
    }

    fn claude_json(presets: &[&str]) -> AgentProfile {
        let command = AgentCommand::Claude {
            extra_args: Vec::new(),
            output_format: AgentOutputFormat::StreamJson,
        };
        coded(command, presets)
    }

    /// An absolute folder path on this OS (under the temp folder) and its normalized form.
    fn folder_at(rel: &str) -> (String, PathBuf) {
        let spelled = format!("{}/{rel}", std::env::temp_dir().display());
        let normalized =
            engram_dashboard_platform::fs::normalize_folder(Path::new(&spelled)).unwrap();
        (spelled, normalized)
    }

    fn codes(resolution: &Resolution) -> Vec<&str> {
        resolution
            .problems
            .iter()
            .map(|problem| match problem {
                BlockReason::Missing { reason, .. }
                | BlockReason::Broken { reason, .. }
                | BlockReason::Conflict { reason, .. }
                | BlockReason::LibraryUnavailable { reason } => reason.code.as_str(),
            })
            .collect()
    }

    fn library_unavailable(message: &str) -> BlockReason {
        BlockReason::LibraryUnavailable {
            reason: Reason {
                code: LIBRARY_UNAVAILABLE.to_owned(),
                args: json!({ "message": message }),
                line: None,
            },
        }
    }

    #[test]
    fn resolve_preset_passes_the_first_scan_and_stats_its_folders_and_the_root() {
        let store = TestStore::new(&["/lib"]);
        let ops = Arc::new(CountOps::default());
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), None, io);
        let (spelled, folder) = folder_at("work");
        let mem = store.mem("/lib");
        mem.file(
            "P.yaml",
            &format!(
                "kind: preset\nversion: 1\ninstructions:\n  - a.md\nfolders:\n  - '{spelled}'\n"
            ),
        );
        mem.file("a.md", "a");
        let resolution = svc.resolve_preset(&lp("P.yaml"));
        assert_eq!(
            store.scans(),
            [scan("/lib", None)],
            "the first scan, joined"
        );
        assert_eq!(
            resolution.instructions,
            [("a.md".to_owned(), Path::new("/lib").join("a.md"))]
        );
        assert_eq!(resolution.folders[0].status, Some(FolderStatus::Ok));
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert_eq!(
            (ops.stats_of(Path::new("/lib")), ops.stats_of(&folder)),
            (1, 1)
        );
        svc.resolve_preset(&lp("P.yaml"));
        assert_eq!(store.scans(), [scan("/lib", None)], "no second scan");
    }

    #[test]
    fn resolve_agent_answers_an_agent_with_a_settings_check_and_reads_the_builtins_once() {
        let dir = crate::builtin::tests::full();
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - a.md\n",
        );
        store.mem("/lib").file("a.md", "a");
        let claude = claude_json(&["P.yaml"]);
        let shell = agent(&["P.yaml"]);
        let stranger = agent(&[]);
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(
            Some("/lib"),
            &store,
            vec![claude.clone(), shell.clone()],
            Some(dir.path()),
            io,
        );
        let resolution = svc.resolve_agent(claude.id).expect("a claude agent");
        assert!(
            resolution.include_priming,
            "the built-in default is attached"
        );
        assert_eq!(resolution.instructions[0].0, "a.md");
        assert_eq!(
            resolution.raw_settings.len(),
            1,
            "the built-in settings' claude field"
        );
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        // Read once, in `new`: later changes to the files do not reach the service.
        std::fs::remove_file(dir.path().join(crate::builtin::file_of(BuiltinId::Priming))).unwrap();
        std::fs::write(
            dir.path()
                .join(crate::builtin::file_of(BuiltinId::DefaultSettings)),
            "kind: settings
version: 1
claude: {changed: true}
",
        )
        .unwrap();
        assert_eq!(svc.resolve_agent(claude.id).as_ref(), Some(&resolution));
        assert_eq!(
            svc.resolve_agent(shell.id),
            None,
            "no settings check — no preset"
        );
        assert_eq!(svc.resolve_agent(stranger.id), None, "not in the registry");
        let all = svc.resolve_agents(&[shell.id, claude.id, stranger.id]);
        let ids: Vec<AgentId> = all.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [shell.id, claude.id, stranger.id]);
        assert!(all[0].1.is_none() && all[2].1.is_none());
        assert_eq!(all[1].1.as_ref(), Some(&resolution));
    }

    #[test]
    fn resolve_agents_spends_one_read_budget_on_every_held_folder_and_stats_each_once() {
        let (one, held_one) = folder_at("held-one");
        let (two, held_two) = folder_at("held-two");
        let store = TestStore::new(&["/lib"]);
        let mem = store.mem("/lib");
        mem.file(
            "P.yaml",
            &format!("kind: preset\nversion: 1\nfolders:\n  - '{one}'\n  - '{two}'\n"),
        );
        mem.file(
            "Q.yaml",
            &format!("kind: preset\nversion: 1\nfolders:\n  - '{one}'\n"),
        );
        let agents = [
            claude_json(&["P.yaml"]),
            claude_json(&["Q.yaml"]),
            claude_json(&["Q.yaml", "P.yaml"]),
        ];
        let ops = Arc::new(CountOps {
            held: vec![held_one.clone(), held_two.clone()],
            ..CountOps::default()
        });
        let limit = Duration::from_secs(1);
        let io = BoundedIo::new(
            limit,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, agents.to_vec(), None, io);
        svc.first_scan_join().unwrap();
        let ids: Vec<AgentId> = agents.iter().map(|profile| profile.id).collect();
        let started = Instant::now();
        let all = svc.resolve_agents(&ids);
        let spent = started.elapsed();
        ops.release();
        // One budget for all three agents; a budget per agent would wait at least three limits.
        assert!(spent < limit + Duration::from_secs(1), "{spent:?}");
        assert_eq!((ops.stats_of(&held_one), ops.stats_of(&held_two)), (1, 1));
        for (id, resolution) in &all {
            let resolution = resolution.as_ref().expect("a claude agent");
            assert!(!resolution.problems.is_empty(), "{id}");
            for problem in &resolution.problems {
                let BlockReason::Missing { reason, .. } = problem else {
                    panic!("{problem:?}")
                };
                assert_eq!(reason.code, resolve::FOLDER_NOT_FOUND);
                assert_eq!(reason.args["why"], folder::TIMEOUT);
            }
        }
        assert_eq!(
            codes(all[2].1.as_ref().unwrap()).len(),
            2,
            "each folder once"
        );
    }

    #[test]
    fn a_library_preset_without_a_usable_root_is_block_four_and_the_root_is_stat_ed_only_for_one() {
        // A root that stats as missing after a good first scan: the stale catalog's preset lists a missing folder.
        let (spelled, folder) = folder_at("gone");
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file(
            "P.yaml",
            &format!("kind: preset\nversion: 1\nfolders:\n  - '{spelled}'\n"),
        );
        let ops = Arc::new(CountOps {
            statuses: HashMap::from([
                (PathBuf::from("/lib"), FolderStatus::Missing),
                (folder.clone(), FolderStatus::Missing),
            ]),
            ..CountOps::default()
        });
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let library = claude_json(&["P.yaml"]);
        let builtin_only = claude_json(&[]);
        let svc = service_io(
            Some("/lib"),
            &store,
            vec![library.clone(), builtin_only.clone()],
            None,
            io,
        );
        assert!(svc
            .resolve_agent(builtin_only.id)
            .unwrap()
            .problems
            .is_empty());
        assert_eq!(
            ops.stats_of(Path::new("/lib")),
            0,
            "no library preset — not asked"
        );
        assert_eq!(
            svc.resolve_agent(library.id).unwrap(),
            Resolution {
                problems: vec![library_unavailable("missing")],
                ..Resolution::default()
            },
            "block ④ alone — no folder, no folder row"
        );
        assert_eq!(ops.stats_of(Path::new("/lib")), 1);
        assert_eq!(
            ops.stats_of(&folder),
            0,
            "block ④'s folders are not stat'ed"
        );

        // No root at all: block ④ alone — the empty catalog's not-found row for the preset is not shown.
        let (unset, _) = service_on(&["/lib"], None);
        let resolution = unset.resolve_preset(&lp("P.yaml"));
        assert_eq!(resolution.problems, [library_unavailable(gate::NO_ROOT)]);

        // A first scan that could not be applied answers its reason alone, with no root stat; the preset under a link
        //   gets no path-through-link row.
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").link("lnk");
        let ops = Arc::new(CountOps::default());
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), None, io);
        *lock(&store.fail_next_scan) = true;
        let resolution = svc.resolve_preset(&lp("lnk/P.yaml"));
        let [BlockReason::LibraryUnavailable { reason }] = resolution.problems.as_slice() else {
            panic!("{:?}", resolution.problems)
        };
        let message = reason.args["message"].as_str().unwrap();
        assert!(!message.is_empty() && message != gate::NO_ROOT, "{message}");
        assert_eq!(ops.stats_of(Path::new("/lib")), 0);
    }

    #[test]
    fn a_settings_rows_state_shows_a_refusal_in_any_backends_field() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "s.yaml",
            "kind: settings\nversion: 1\nclaude: {}\ncodex:\n  - foo\n",
        );
        let rows = svc.list(None).unwrap().rows;
        let state = rows[0].state.as_ref().expect("a refusal");
        assert_eq!(
            (state.code.as_str(), state.line),
            ("codex-config-line-invalid", Some(4))
        );
    }

    #[test]
    fn a_reference_through_a_link_inside_the_library_is_path_through_link() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.link("lnk");
        mem.file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - lnk/x.md\n  - gone.md\n",
        );
        let resolution = svc.resolve_preset(&lp("P.yaml"));
        assert_eq!(
            resolution.problems,
            [
                BlockReason::Broken {
                    item: "lnk/x.md".to_owned(),
                    via: vec!["P.yaml".to_owned()],
                    reason: Reason {
                        code: PATH_THROUGH_LINK.to_owned(),
                        args: json!({ "path": "lnk/x.md" }),
                        line: Some(4),
                    },
                },
                BlockReason::Missing {
                    item: "gone.md".to_owned(),
                    via: vec!["P.yaml".to_owned()],
                    reason: Reason {
                        code: NOT_FOUND.to_owned(),
                        args: json!({ "path": "gone.md" }),
                        line: Some(5),
                    },
                },
            ]
        );
    }

    #[test]
    fn an_attached_preset_path_that_does_not_parse_blocks_the_agent() {
        let store = TestStore::new(&["/lib"]);
        let claude = claude_json(&["../outside.yaml"]);
        let svc = service(
            Some("/lib"),
            &store,
            vec![claude.clone()],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let resolution = svc.resolve_agent(claude.id).unwrap();
        let [BlockReason::Broken { item, via, reason }] = resolution.problems.as_slice() else {
            panic!("{:?}", resolution.problems)
        };
        assert_eq!((item.as_str(), via.len()), ("../outside.yaml", 0));
        assert_eq!(reason.code, "item-path-invalid");
        assert_eq!(reason.args["path"], "../outside.yaml");
        assert!(resolution.blocked().is_some());
    }

    #[test]
    fn a_codex_agents_mode_decides_whether_its_skills_are_judged() {
        let store = TestStore::new(&["/lib"]);
        store
            .mem("/lib")
            .file("P.yaml", "kind: preset\nversion: 1\nskills:\n  - s\n");
        store
            .mem("/lib")
            .file("s/SKILL.md", "---\nname: s\ndescription: \"\"\n---\n");
        let codex = |mode| {
            coded(
                AgentCommand::Codex {
                    extra_args: Vec::new(),
                    output_format: mode,
                },
                &["P.yaml"],
            )
        };
        let json_agent = codex(AgentOutputFormat::StreamJson);
        let terminal_agent = codex(AgentOutputFormat::Terminal);
        let svc = service(
            Some("/lib"),
            &store,
            vec![json_agent.clone(), terminal_agent.clone()],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let json_mode = svc.resolve_agent(json_agent.id).unwrap();
        assert_eq!(codes(&json_mode), ["codex-skill-description-empty"]);
        let terminal = svc.resolve_agent(terminal_agent.id).unwrap();
        assert!(terminal.problems.is_empty(), "{:?}", terminal.problems);
        let notices: Vec<&str> = terminal.notices.iter().map(|n| n.code.as_str()).collect();
        assert_eq!(
            notices,
            [
                engram_dashboard_agent::launch::BUILTIN_UNAVAILABLE,
                "codex-terminal-omitted"
            ],
            "the default preset's stand-in (no built-in folder) is told, and the skill is named as left out"
        );
    }

    // ── the root setting: status · set_root · first start ────────────────────────────────────────────────

    /// A service whose `library.json` lives in a real temp folder (`<temp>/state`). The test store's roots are
    /// absolute paths under the temp folder (named by [`OnDisk::at`]); nothing of them is on disk unless a test makes
    /// it, and their stats answer from `ops` (default `Ok`).
    struct OnDisk {
        temp: tempfile::TempDir,
        store: Arc<TestStore>,
        ops: Arc<StatOps>,
        default_root: PathBuf,
    }

    impl OnDisk {
        /// `roots` = the test store's roots by name; `default` = the default place's name; `config` = the
        /// `library.json` text (`None` = no file).
        fn new(roots: &[&str], default: &str, config: Option<&str>) -> OnDisk {
            let temp = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(temp.path().join("state")).unwrap();
            let at = |name: &str| normalize_folder(&temp.path().join(name)).unwrap();
            let names: Vec<String> = roots
                .iter()
                .map(|name| at(name).display().to_string())
                .collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let disk = OnDisk {
                default_root: at(default),
                store: TestStore::new(&names),
                ops: Arc::new(StatOps::default()),
                temp,
            };
            if let Some(text) = config {
                std::fs::write(disk.config_path(), text).unwrap();
            }
            disk
        }

        fn at(&self, name: &str) -> PathBuf {
            normalize_folder(&self.temp.path().join(name)).unwrap()
        }

        fn mem(&self, name: &str) -> &MemStore {
            self.store.at_root(&self.at(name))
        }

        fn state(&self) -> PathBuf {
            self.temp.path().join("state")
        }

        fn config_path(&self) -> PathBuf {
            self.state().join(config::LIBRARY_CONFIG_FILE)
        }

        /// `library.json` as JSON; `None` = no file.
        fn config(&self) -> Option<serde_json::Value> {
            let text = std::fs::read_to_string(self.config_path()).ok()?;
            Some(serde_json::from_str(&text).unwrap())
        }

        fn explicit(&self, name: &str) -> String {
            json!({ "schema_version": 1, "root": self.at(name).to_str().unwrap() }).to_string()
        }

        /// A new service over a fresh `config::load` of the state folder — one daemon start, at [`ROOT_BASE`].
        fn start(&self) -> Arc<LibraryService> {
            self.start_at(ROOT_BASE)
        }

        /// [`OnDisk::start`] at the root generation base `base`.
        fn start_at(&self, base: u64) -> Arc<LibraryService> {
            let io = BoundedIo::new(
                IO_LIMIT,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::clone(&self.ops) as Arc<dyn FolderOps>,
                Arc::new(ThreadSpawn),
            );
            let values = LibraryValues {
                registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(Vec::new())))),
                backends: Arc::new(AgentBackends),
                config: config::load(&self.state()),
                default_root: self.default_root.clone(),
                daemon_id: "daemon-1".to_owned(),
                root_generation_base: base,
                builtin_dir: None,
                launch_root: PathBuf::from("/launch"),
                home: None,
                env: Arc::new(|_| None),
            };
            let store: Arc<dyn LibraryStore> = Arc::clone(&self.store) as Arc<dyn LibraryStore>;
            Arc::new(LibraryService::new(values, store, io))
        }
    }

    fn refused(refusal: LibraryRefusal) -> (RefusalClass, Reason) {
        match refusal {
            LibraryRefusal::Refused { class, reason } => (class, reason),
            LibraryRefusal::Store(e) => panic!("not a library refusal: {e}"),
        }
    }

    fn word(disk: &OnDisk) -> serde_json::Value {
        disk.config().expect("library.json")["root"].clone()
    }

    #[test]
    fn a_status_answers_the_daemon_id_root_setting_state_config_suggestion_and_generation() {
        let disk = OnDisk::new(&["lib"], "library", None);
        let svc = disk.start();
        let base = svc.numbers().root;
        assert_eq!(base, ROOT_BASE, "the run starts at the base it was given");
        assert_eq!(
            svc.status(),
            LibraryStatus {
                daemon_id: "daemon-1".to_owned(),
                root: None,
                root_setting: Some(RootSettingKind::Default),
                state: RootStatus::Unset,
                config: ConfigHealth::Ok,
                suggested_new: disk.default_root.clone(),
                root_generation: base,
            },
            "a missing library.json holds the default value, with no root before the first start"
        );
        lock(&disk.ops.0).insert(disk.at("lib"), FolderStatus::NotADirectory);
        let refusal = refused(svc.set_root(&disk.at("lib"), false).unwrap_err()).1;
        assert_eq!(refusal.code, LIBRARY_ROOT_UNUSABLE);
        lock(&disk.ops.0).clear();
        let status = svc.set_root(&disk.at("lib"), false).unwrap();
        assert_eq!(
            (
                status.root,
                status.root_setting,
                status.state,
                status.root_generation
            ),
            (
                Some(disk.at("lib")),
                Some(RootSettingKind::Explicit),
                RootStatus::Ok,
                base + 1
            )
        );
        lock(&disk.ops.0).insert(
            disk.at("lib"),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        // The stat worker of the status `set_root` answered may still be in flight, and a required ask joins it
        //   (`folder.rs` — one worker per key): ask until it has ended.
        let deadline = Instant::now() + Duration::from_secs(10);
        let state = loop {
            let state = svc.status().state;
            if state != RootStatus::Ok || Instant::now() > deadline {
                break state;
            }
            thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(
            state,
            RootStatus::Missing {
                why: "denied".to_owned()
            },
            "every stat answer but Ok is missing, with why"
        );
    }

    #[test]
    fn set_root_writes_an_explicit_path_publishes_it_and_answers_the_status_after() {
        let disk = OnDisk::new(&["lib", "lib2"], "library", None);
        disk.mem("lib2").file("b.md", "b");
        let svc = disk.start();
        let status = svc.set_root(&disk.at("lib2"), false).unwrap();
        assert_eq!(word(&disk), json!(disk.at("lib2").to_str().unwrap()));
        assert_eq!(status.root, Some(disk.at("lib2")));
        assert_eq!(status.root_setting, Some(RootSettingKind::Explicit));
        assert_eq!(status.config, ConfigHealth::Ok);
        assert_eq!(paths(&svc.list(None).unwrap()), ["b.md"]);
        assert_eq!(
            disk.start().status().root,
            Some(disk.at("lib2")),
            "the next start reads it"
        );
    }

    #[test]
    fn set_root_to_the_default_place_stores_the_default_value_however_it_is_spelled() {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        let svc = disk.start();
        svc.set_root(&disk.at("lib2"), false).unwrap();
        let spelled = disk.temp.path().join("state").join("..").join("library");
        let status = svc.set_root(&spelled, false).unwrap();
        assert_eq!(word(&disk), json!("default"), "not the absolute path");
        assert_eq!(status.root_setting, Some(RootSettingKind::Default));
        assert_eq!(status.root, Some(disk.at("library")));
    }

    #[test]
    fn set_root_refuses_a_path_that_is_not_absolute_and_writes_nothing() {
        let disk = OnDisk::new(&["lib"], "library", None);
        let svc = disk.start();
        let (class, reason) = refused(svc.set_root(Path::new("relative/lib"), true).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::InvalidArgument, "item-path-invalid")
        );
        assert!(disk.config().is_none());
        assert!(!Path::new("relative").exists());
    }

    #[test]
    fn set_root_on_a_read_only_config_refuses_before_creating_anything() {
        let newer = r#"{"schema_version": 2, "root": "default"}"#;
        for unreadable in [false, true] {
            let disk = OnDisk::new(&["lib", "new"], "library", None);
            match unreadable {
                false => std::fs::write(disk.config_path(), newer).unwrap(),
                true => std::fs::create_dir(disk.config_path()).unwrap(),
            }
            let svc = disk.start();
            let (class, reason) = refused(svc.set_root(&disk.at("new"), true).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_CONFIG_READ_ONLY),
                "unreadable: {unreadable}"
            );
            assert!(
                unreadable || reason.args["why"].as_str().unwrap().contains("newer"),
                "{reason:?}"
            );
            assert!(!disk.at("new").exists(), "nothing was created");
            match unreadable {
                false => assert_eq!(std::fs::read_to_string(disk.config_path()).unwrap(), newer),
                true => assert!(disk.config_path().is_dir()),
            }
            let status = svc.status();
            assert!(
                matches!(status.config, ConfigHealth::ReadOnly { .. }),
                "{status:?}"
            );
            assert_eq!((status.root_setting, status.root), (None, None));
            assert_eq!(svc.first_start(), FirstStart::NotDue);
        }
    }

    #[test]
    fn set_root_with_create_makes_the_folder_and_its_ancestors() {
        let disk = OnDisk::new(&["a/b/c"], "library", None);
        let svc = disk.start();
        svc.set_root(&disk.at("a/b/c"), true).unwrap();
        assert!(disk.at("a/b/c").is_dir());
        assert_eq!(svc.status().root, Some(disk.at("a/b/c")));
    }

    #[test]
    fn a_set_root_with_create_whose_create_fails_is_root_unusable_with_nothing_published_or_written(
    ) {
        let disk = OnDisk::new(&["blocker/lib"], "library", None);
        std::fs::write(disk.temp.path().join("blocker"), "a file").unwrap();
        let svc = disk.start();
        let (class, reason) = refused(svc.set_root(&disk.at("blocker/lib"), true).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Unavailable, LIBRARY_ROOT_UNUSABLE)
        );
        assert_eq!(
            reason.args["path"],
            json!(disk.at("blocker/lib").display().to_string())
        );
        assert!(disk.config().is_none(), "nothing written");
        assert_eq!(svc.status().root, None, "nothing published");
        assert_eq!(svc.numbers().root, ROOT_BASE, "no root change ran");
    }

    #[test]
    fn a_set_root_whose_stat_is_refused_publishes_nothing_and_writes_nothing() {
        for old in [None, Some("lib")] {
            let disk = OnDisk::new(&["lib", "gone"], "library", None);
            if let Some(old) = old {
                std::fs::write(disk.config_path(), disk.explicit(old)).unwrap();
            }
            let svc = disk.start();
            let before = svc.status();
            lock(&disk.ops.0).insert(disk.at("gone"), FolderStatus::Missing);
            let (class, reason) = refused(svc.set_root(&disk.at("gone"), false).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_ROOT_UNUSABLE),
                "{old:?}"
            );
            assert_eq!(
                reason.args,
                json!({ "path": disk.at("gone").display().to_string(), "why": reason.args["why"] }),
                "{old:?}: path and why only"
            );
            assert!(
                reason.args["why"].as_str().unwrap().contains("missing"),
                "{old:?}: {reason:?}"
            );
            match old {
                None => assert!(disk.config().is_none(), "no file is written"),
                Some(old) => assert_eq!(
                    std::fs::read_to_string(disk.config_path()).unwrap(),
                    disk.explicit(old),
                    "the file keeps its setting"
                ),
            }
            let after = svc.status();
            assert_eq!(
                (
                    after.root,
                    after.root_setting,
                    after.config,
                    after.root_generation
                ),
                (
                    before.root,
                    before.root_setting,
                    before.config,
                    before.root_generation
                ),
                "{old:?}: nothing published, the state as before"
            );
        }
    }

    #[test]
    fn a_set_root_whose_walk_fails_at_the_root_leaves_library_json_as_it_was() {
        let disk = OnDisk::new(&["lib", "lib2"], "library", None);
        let old = disk.explicit("lib");
        std::fs::write(disk.config_path(), &old).unwrap();
        disk.mem("lib").file("a.md", "a");
        let svc = disk.start();
        svc.list(None).unwrap();
        disk.mem("lib2").fail_listing("", ListFailure::Open);
        let reason = refused(svc.set_root(&disk.at("lib2"), false).unwrap_err()).1;
        assert_eq!(reason.code, LIBRARY_ROOT_UNUSABLE);
        assert_eq!(
            std::fs::read_to_string(disk.config_path()).unwrap(),
            old,
            "the exact old bytes"
        );
        assert_eq!(svc.status().root, Some(disk.at("lib")));
        assert_eq!(svc.numbers().root, ROOT_BASE);
        assert_eq!(paths(&svc.list(None).unwrap()), ["a.md"]);
    }

    #[test]
    fn a_library_json_write_refused_or_failed_after_the_publish_leaves_the_new_root_live_and_unsaved(
    ) {
        let default_doc = r#"{"schema_version": 1, "root": "default"}"#;
        for judged in [true, false] {
            let disk = OnDisk::new(&["library", "lib2"], "library", Some(default_doc));
            disk.mem("lib2").file("b.md", "b");
            let svc = disk.start();
            match judged {
                // Unreadable now (a folder in its place): the judge before the write refuses.
                true => {
                    std::fs::remove_file(disk.config_path()).unwrap();
                    std::fs::create_dir(disk.config_path()).unwrap();
                }
                false => svc.config.fail_writes(Some("disk full")),
            }
            let (class, reason) = refused(svc.set_root(&disk.at("lib2"), false).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_CONFIG_WRITE_FAILED),
                "judged: {judged}"
            );
            let why = reason.args["why"].as_str().unwrap();
            assert!(why.contains("live but not saved"), "{why}");
            assert!(judged || why.contains("disk full"), "{why}");
            match judged {
                true => assert!(disk.config_path().is_dir(), "left as it was"),
                false => assert_eq!(
                    std::fs::read_to_string(disk.config_path()).unwrap(),
                    default_doc,
                    "the file keeps its setting"
                ),
            }
            let status = svc.status();
            assert_eq!(
                (
                    status.root.clone(),
                    status.root_setting,
                    status.root_generation
                ),
                (
                    Some(disk.at("lib2")),
                    Some(RootSettingKind::Explicit),
                    ROOT_BASE + 1
                ),
                "judged: {judged}: the new root is live and nothing was undone"
            );
            assert!(
                matches!(&status.config, ConfigHealth::Unsaved { why } if !why.is_empty()),
                "{status:?}"
            );
            assert_eq!(paths(&svc.list(None).unwrap()), ["b.md"]);
            if !judged {
                assert_eq!(
                    disk.start().status().root,
                    Some(disk.at("library")),
                    "the next start follows the file that is left"
                );
                svc.config.fail_writes(None);
                let status = svc.set_root(&disk.at("lib2"), false).unwrap();
                assert_eq!(
                    status.config,
                    ConfigHealth::Ok,
                    "a later save clears unsaved"
                );
                assert_eq!(word(&disk), json!(disk.at("lib2").to_str().unwrap()));
            }
        }
    }

    #[test]
    fn a_failed_save_over_a_corrupt_file_is_unsaved_not_corrupt_and_the_next_start_follows_the_corrupt_file(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", Some("not json"));
        let svc = disk.start();
        svc.config.fail_writes(Some("disk full"));
        let reason = refused(svc.set_root(&disk.at("lib2"), false).unwrap_err()).1;
        assert_eq!(reason.code, LIBRARY_CONFIG_WRITE_FAILED);
        assert_eq!(
            std::fs::read_to_string(disk.config_path()).unwrap(),
            "not json"
        );
        let status = svc.status();
        assert!(
            matches!(&status.config, ConfigHealth::Unsaved { .. }),
            "the live state wins: {status:?}"
        );
        assert_eq!(
            (status.root, status.root_setting),
            (Some(disk.at("lib2")), Some(RootSettingKind::Explicit)),
            "root and setting describe the live root"
        );
        let next = disk.start().status();
        assert_eq!(
            (next.root, next.root_setting, next.config),
            (None, None, ConfigHealth::Corrupt)
        );
    }

    /// Waits (at most 10 s) until a root-change take has found the mutex held.
    fn wait_blocked(svc: &LibraryService) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while svc
            .root_change_blocked
            .load(std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            assert!(
                Instant::now() < deadline,
                "no set_root waited on the root-change mutex"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn set_root_calls_serialize_across_the_root_change_and_the_library_json_write() {
        let disk = OnDisk::new(&["lib", "a", "b"], "library", None);
        std::fs::write(disk.config_path(), disk.explicit("lib")).unwrap();
        let svc = disk.start();
        svc.list(None).unwrap();
        let (a, b) = (disk.at("a"), disk.at("b"));
        disk.store.hold_at(At::Start, a.to_str().unwrap());
        let first = {
            let a = a.clone();
            on_thread(&svc, move |svc| svc.set_root(&a, false))
        };
        disk.store.wait_held();
        disk.store.hold_at(At::Start, b.to_str().unwrap());
        let second = {
            let b = b.clone();
            on_thread(&svc, move |svc| svc.set_root(&b, false))
        };
        wait_blocked(&svc);
        svc.gate
            .wait_for("only the first root change is in line", |line, _| {
                line == [("root", true)]
            });
        assert_eq!(
            word(&disk),
            json!(disk.at("lib").to_str().unwrap()),
            "nothing is written before a publish"
        );
        disk.store.release();
        join(first).unwrap();
        assert_eq!(
            word(&disk),
            json!(a.to_str().unwrap()),
            "the first wrote after its publish"
        );
        disk.store.wait_held();
        assert_eq!(
            word(&disk),
            json!(a.to_str().unwrap()),
            "the second walks only after the first's write"
        );
        disk.store.release();
        join(second).unwrap();
        assert_eq!(word(&disk), json!(b.to_str().unwrap()));
        assert_eq!(svc.status().root, Some(b));
    }

    // ── first start ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_first_start_creates_the_default_place_writes_the_default_value_and_publishes_it() {
        let disk = OnDisk::new(&["library"], "library", None);
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::Published);
        assert!(disk.at("library").is_dir());
        assert_eq!(
            disk.config(),
            Some(json!({ "schema_version": 1, "root": "default" })),
            "the default value, not the path"
        );
        let status = svc.status();
        assert_eq!(
            (
                status.root,
                status.root_setting,
                status.state,
                status.config
            ),
            (
                Some(disk.at("library")),
                Some(RootSettingKind::Default),
                RootStatus::Ok,
                ConfigHealth::Ok
            )
        );
        assert!(svc.list(None).unwrap().rows.is_empty());
        assert_eq!(svc.first_start(), FirstStart::NotDue, "once");
        assert_eq!(
            disk.start().first_start(),
            FirstStart::NotDue,
            "the next start"
        );
    }

    #[test]
    fn an_uncreatable_default_place_leaves_the_root_unset_and_no_file_and_the_next_start_retries() {
        let disk = OnDisk::new(&["blocker/library"], "blocker/library", None);
        std::fs::write(disk.temp.path().join("blocker"), "a file").unwrap();
        let svc = disk.start();
        assert!(matches!(svc.first_start(), FirstStart::Failed(_)));
        assert!(disk.config().is_none());
        let status = svc.status();
        assert_eq!(
            (
                status.root,
                status.root_setting,
                status.state,
                status.config
            ),
            (
                None,
                Some(RootSettingKind::Default),
                RootStatus::Unset,
                ConfigHealth::Ok
            )
        );
        std::fs::remove_file(disk.temp.path().join("blocker")).unwrap();
        let next = disk.start();
        assert_eq!(next.first_start(), FirstStart::Published);
        assert_eq!(next.status().root, Some(disk.at("blocker/library")));
    }

    #[test]
    fn a_first_start_whose_root_change_is_refused_writes_nothing_and_the_next_start_retries() {
        let disk = OnDisk::new(&["library"], "library", None);
        lock(&disk.ops.0).insert(
            disk.at("library"),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        let svc = disk.start();
        assert!(matches!(svc.first_start(), FirstStart::Failed(why) if why.contains("denied")));
        assert!(disk.config().is_none());
        assert_eq!(svc.status().root, None);
        assert_eq!(svc.numbers().root, ROOT_BASE);
        lock(&disk.ops.0).clear();
        assert_eq!(disk.start().first_start(), FirstStart::Published);
    }

    #[test]
    fn a_first_start_whose_write_fails_keeps_the_default_root_live_unsaved_and_the_next_start_runs_it_again(
    ) {
        let disk = OnDisk::new(&["library"], "library", None);
        let svc = disk.start();
        svc.config.fail_writes(Some("disk full"));
        assert!(matches!(svc.first_start(), FirstStart::Unsaved(why) if why.contains("disk full")));
        assert!(disk.config().is_none(), "no file");
        let status = svc.status();
        assert_eq!(
            (
                status.root.clone(),
                status.root_setting,
                status.state.clone()
            ),
            (
                Some(disk.at("library")),
                Some(RootSettingKind::Default),
                RootStatus::Ok
            ),
            "the default root is live this run"
        );
        assert!(
            matches!(&status.config, ConfigHealth::Unsaved { .. }),
            "{status:?}"
        );
        assert_eq!(svc.first_start(), FirstStart::NotDue, "once a run");
        let next = disk.start();
        assert_eq!(
            next.first_start(),
            FirstStart::Published,
            "no file is left: the first start runs again"
        );
        assert_eq!(
            disk.config(),
            Some(json!({ "schema_version": 1, "root": "default" }))
        );
        assert_eq!(next.status().root, Some(disk.at("library")));
    }

    #[test]
    fn a_first_start_that_finds_a_newer_file_at_its_write_stays_live_and_the_next_start_is_read_only(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        let svc = disk.start();
        let newer = r#"{"schema_version": 9, "root": "default"}"#;
        std::fs::write(disk.config_path(), newer).unwrap();
        assert!(matches!(svc.first_start(), FirstStart::Unsaved(why) if why.contains("newer")));
        assert_eq!(std::fs::read_to_string(disk.config_path()).unwrap(), newer);
        assert_eq!(svc.status().root, Some(disk.at("library")));
        let next = disk.start();
        assert_eq!(next.first_start(), FirstStart::NotDue);
        let status = next.status();
        assert_eq!(status.root, None);
        assert!(
            matches!(&status.config, ConfigHealth::ReadOnly { .. }),
            "{status:?}"
        );
        let reason = refused(next.set_root(&disk.at("lib2"), false).unwrap_err()).1;
        assert_eq!(reason.code, LIBRARY_CONFIG_READ_ONLY);
    }

    #[test]
    fn a_configured_root_that_vanished_is_missing_not_recreated_and_the_setting_is_unchanged() {
        for root in ["library", "chosen"] {
            let disk = OnDisk::new(&["library", "chosen"], "library", None);
            let config = match root {
                "library" => r#"{"schema_version": 1, "root": "default"}"#.to_owned(),
                _ => disk.explicit(root),
            };
            std::fs::write(disk.config_path(), &config).unwrap();
            lock(&disk.ops.0).insert(disk.at(root), FolderStatus::Missing);
            let svc = disk.start();
            assert_eq!(svc.first_start(), FirstStart::NotDue, "{root}");
            let status = svc.status();
            assert_eq!(status.root, Some(disk.at(root)), "{root}");
            assert_eq!(
                status.state,
                RootStatus::Missing {
                    why: "missing".to_owned()
                },
                "{root}"
            );
            assert!(!disk.at(root).exists(), "{root}: never re-created");
            assert_eq!(
                std::fs::read_to_string(disk.config_path()).unwrap(),
                config,
                "{root}"
            );
            let setting = match root {
                "library" => RootSettingKind::Default,
                _ => RootSettingKind::Explicit,
            };
            assert_eq!(status.root_setting, Some(setting), "{root}");
        }
    }

    #[test]
    fn a_corrupt_config_is_no_first_start_and_a_set_root_over_it_copies_it_aside_and_makes_it_ok() {
        let disk = OnDisk::new(&["library", "lib2"], "library", Some("not json"));
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::NotDue);
        let status = svc.status();
        assert_eq!(
            (status.root, status.root_setting, status.config),
            (None, None, ConfigHealth::Corrupt)
        );
        let status = svc.set_root(&disk.at("lib2"), false).unwrap();
        assert_eq!(status.config, ConfigHealth::Ok);
        assert_eq!(status.root_setting, Some(RootSettingKind::Explicit));
        assert_eq!(
            std::fs::read_to_string(disk.state().join("library.json.corrupt")).unwrap(),
            "not json"
        );
    }

    // ── the root generation on a change ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_stale_root_generation_is_refused_inside_the_changes_own_lock_take_even_when_queued_behind_a_root_change(
    ) {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        svc.list(None).unwrap();
        let given = svc.status().root_generation;
        store.hold_at(At::Start, "/lib2");
        let root_change = on_thread(&svc, |svc| svc.change_root(PathBuf::from("/lib2")));
        store.wait_held();
        let ran = Arc::new(Mutex::new(false));
        let edit = {
            let ran = Arc::clone(&ran);
            on_thread(&svc, move |svc| {
                svc.change_at(Some(given), |_, _, _| {
                    *lock(&ran) = true;
                    ((), Vec::new())
                })
            })
        };
        svc.gate
            .wait_for("the edit is queued behind the root change", |line, _| {
                line == [("root", true), ("change", false)]
            });
        store.release();
        join(root_change).unwrap();
        let changes = svc.numbers().change;
        let (class, reason) = refused(join(edit).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str(), reason.args.clone()),
            (
                RefusalClass::Conflict,
                ROOT_GENERATION_STALE,
                json!({ "given": given, "current": given + 1 })
            )
        );
        assert!(!*lock(&ran), "nothing applied");
        assert_eq!(svc.numbers().change, changes, "no generation moved");
        let current = svc.status().root_generation;
        let root = svc.change_at(Some(current), |_, root, _| {
            (root.map(Path::to_path_buf), Vec::new())
        });
        assert_eq!(root.unwrap(), Some(PathBuf::from("/lib2")));
        assert!(svc.change_at(None, |_, _, _| ((), Vec::new())).is_ok());
        assert_eq!(svc.numbers().change, changes + 2);
    }

    #[test]
    fn a_generation_kept_from_an_earlier_run_with_another_base_is_refused() {
        let disk = OnDisk::new(&["lib", "lib2"], "library", None);
        let first = disk.start_at(ROOT_BASE);
        let kept = first
            .set_root(&disk.at("lib"), false)
            .unwrap()
            .root_generation;
        let next = disk.start_at(ROOT_BASE + 50_000);
        assert_eq!(next.status().root_generation, ROOT_BASE + 50_000);
        let moved = next
            .set_root(&disk.at("lib2"), false)
            .unwrap()
            .root_generation;
        assert_eq!(moved, ROOT_BASE + 50_001, "one root change in each run");
        let (class, reason) = refused(
            next.change_at(Some(kept), |_, _, _| ((), Vec::new()))
                .unwrap_err(),
        );
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Conflict, ROOT_GENERATION_STALE),
            "a generation kept from the earlier run is stale"
        );
        assert!(next
            .change_at(Some(moved), |_, _, _| ((), Vec::new()))
            .is_ok());
    }

    // ── folders · read ───────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn folders_answers_every_folder_by_path_and_marks_skill_folders() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file("rules/a.md", "a");
        mem.file("deep/er/x.md", "x");
        mem.file(
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: does s\n---\nbody\n",
        );
        let answer = svc.folders().unwrap();
        let tree: Vec<(&str, &str, bool)> = answer
            .folders
            .iter()
            .map(|node| (node.path.as_str(), node.name.as_str(), node.skill))
            .collect();
        assert_eq!(
            tree,
            [
                ("deep", "deep", false),
                ("deep/er", "er", false),
                ("rules", "rules", false),
                ("skills", "skills", false),
                ("skills/s", "s", true),
            ]
        );
    }

    #[test]
    fn folders_and_read_answer_library_unavailable_with_no_published_root() {
        let (svc, _store) = service_on(&["/lib"], None);
        for refusal in [
            svc.folders().unwrap_err(),
            svc.read(&lp("a.md")).unwrap_err(),
        ] {
            let (class, reason) = refused(refusal);
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
            );
        }
    }

    #[test]
    fn read_answers_the_row_absolute_path_direct_and_indirect_references_excerpt_and_settings_fields(
    ) {
        let store = TestStore::new(&["/lib"]);
        let top_agent = agent(&["top.yaml"]);
        let mut inner_agent = agent(&["inner.yaml"]);
        inner_agent.name = "b".to_owned();
        let svc = service(
            Some("/lib"),
            &store,
            vec![top_agent.clone(), inner_agent.clone()],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let mem = store.mem("/lib");
        let rules = "# Rules\n\nKeep it short.\n";
        mem.file("rules/a.md", rules);
        mem.file(
            "inner.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/a.md\n",
        );
        mem.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - inner.yaml\n",
        );
        mem.file(
            "set.yaml",
            "kind: settings\nversion: 1\nclaude: {}\ncodex: {}\n",
        );

        let a = svc.read(&lp("rules/a.md")).unwrap();
        assert_eq!(a.row.path, "rules/a.md");
        assert_eq!(
            (a.row.kind.clone(), a.row.references),
            (ItemKind::Instruction, 2)
        );
        assert_eq!(a.absolute, lp("rules/a.md").to_path(Path::new("/lib")));
        assert_eq!(a.presets_direct, ["inner.yaml"]);
        assert_eq!(a.presets_indirect, ["top.yaml"]);
        assert!(a.agents_direct.is_empty());
        let mut both = vec![top_agent.id, inner_agent.id];
        both.sort_unstable();
        assert_eq!(a.agents_indirect, both);
        assert_eq!(a.excerpt, Some(excerpt(rules)));
        assert!(a.settings_fields.is_empty());

        let inner = svc.read(&lp("inner.yaml")).unwrap();
        assert_eq!(inner.presets_direct, ["top.yaml"]);
        assert!(inner.presets_indirect.is_empty());
        assert_eq!(inner.agents_direct, [inner_agent.id]);
        assert_eq!(inner.agents_indirect, [top_agent.id]);
        assert_eq!(inner.row.references, 2);
        assert_eq!(inner.excerpt, None);

        let settings = svc.read(&lp("set.yaml")).unwrap();
        assert_eq!(settings.settings_fields, ["claude", "codex"]);

        let (class, reason) = refused(svc.read(&lp("missing.md")).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::NotFound, NOT_FOUND)
        );
        assert_eq!(reason.args, json!({ "path": "missing.md" }));
    }
}
