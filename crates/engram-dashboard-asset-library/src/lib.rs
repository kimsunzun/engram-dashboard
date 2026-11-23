//! # engram-dashboard-asset-library — the preset library
//!
//! The library holds instruction, skill, settings and preset files in the folders of its mounts. This crate reads,
//! checks and resolves them and makes the launch materials; the daemon creates the service, plugs its ports in, passes
//! it values and calls the first start and the boot scan (ADR-0311 · ADR-0382).
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
//! there). Library files in a mount's folder are read with the store directly (local by assumption). Every stat, list,
//! canonicalize, file identity or read of a folder outside the library — folder entries, native skill roots, the working
//! folder, a mount's folder itself and its ancestors (`mounts::overlap`) — goes through
//! [`folder::BoundedIo`], never under the library lock — except a rename's one bounded stat of a mount folder whose file
//! reads as not found inside its hold (`rename::LibraryService::mount_gone`, ADR-0405 decision 3). The built-ins
//! (`Builtins::new`), the launch folder (`prepare_in`, the sweep, `release`), `library.json` (`config.rs`), the
//! folder a mount command's `create` or the first start creates, and the `.git/info/exclude` line of a mount's `.trash`
//! ([`LibraryService::exclude_trash`] — a job of the service's spawner, holding no library lock) use std `fs`.
//!
//! ## The mounts
//!
//! The library is one virtual root whose top holds named mounts, each mapped to a folder (`mounts.rs` — ADR-0396 ·
//! ADR-0400). Every path the service takes, keeps or answers is virtual (`<mount>/…`); it is joined to a mount's folder
//! only at the disk boundary, through `mounts.rs`, and a path whose first name names no registered mount reads as
//! absent. The built-in mount `engram` is always present and read-only: `list`, `folders` and `read` answer it from the
//! built-ins (`builtin.rs`), never from the catalog or a walk, and a write into it refuses [`MOUNT_READ_ONLY`]
//! (ADR-0400 decision 20). A computation that resolves judges each mount it reaches on its own (`resolve.rs` ·
//! `read_states` — ADR-0400 decision 19); what it stats or reads ahead of resolving follows references by the
//! resolution's own tag rule (`resolve::TagRule`). The mount list is persisted in `library.json` (`config.rs` — ADR-0375 ·
//! ADR-0376), read by the daemon before [`LibraryService::new`]; `new` does no disk work beyond the built-ins. The
//! mount commands ([`LibraryService::add_mount`] · [`LibraryService::remove_mount`] ·
//! [`LibraryService::relocate_mount`] · [`LibraryService::rename_mount`]) and [`LibraryService::first_start`] are the
//! only writers of the setting. Add and
//! relocate judge the folder's overlap with the other mounts — only mounts — and stat it on one read budget
//! (`mounts::overlap` — fail closed; a folder `create` makes is made only after the judgement passed), then walk it and
//! publish it under a `Root` ticket (`gate.rs` `Gate::change_mount`); remove drops the mount and its range, touching no
//! file; rename re-points every reference to the mount in the preset files and the agents, re-keys its range and
//! renames it, all in one `Root`-ticket write-lock hold (`rename.rs` · `Gate::rename_mount` — ADR-0400 decision 12);
//! each raises the one root generation (ADR-0400 decisions 9–16). The first start stats the default place and
//! publishes it as the mount `default_library` without a walk (ADR-0382). Each writes `library.json` — every published
//! mount, one at the default place as `"default"` — only after its publish succeeded (ADR-0381): a refused publish
//! writes nothing, and a write that fails leaves the new mounts live and the setting `unsaved` — nothing is undone, and
//! repeating the same command saves again (the list last saved tells a repeat from a mistake — ADR-0400 decision 14).
//! One mount-change mutex serializes them across the publish and the write; under it they take the gate's locks, the
//! leaf mutexes (the config's state, the held overlaps, the load judgement's state — each held for no IO), the
//! registry's read lock under the library read lock for the reference counts, and do bounded folder IO (the new
//! folder's checks, a rename plan's `.git` stats, the final status); a rename also writes preset files, stats the
//! folder of a mount whose file read as not found, and calls the profile-edit port inside its hold. A read-only
//! `library.json` refuses every mount command first. `agent.new
//! {presets}` re-checks the change number its judgement was made at inside the registry's insert
//! ([`LibraryService::change_number`] — a leaf read), so a rename's agents pass either rewrites the new agent or the
//! insert judges again (ADR-0400 decision 18).
//!
//! The daemon never walks the library before it is reachable (ADR-0382): on every start, after the first-start step,
//! it calls [`LibraryService::start_boot_scan`], which runs the first scan on a background job and returns at once.
//! Once per run, before any mount is stat'ed or changed — by that job, a reader, a launch, `status` or a mount command,
//! whichever comes first, the others waiting for it — the mounts are judged against each other: a mount overlapping an
//! earlier one is held — registered, never walked, read as unreadable naming the other — until it is relocated or
//! removed (ADR-0400 decision 10). Catalog readers wait for the scan at the first-scan entrance; launch checks skip the
//! entrance and do not wait.
//! Every scan stats the mounts it would walk in one bounded batch before it draws its ticket; a mount that does not stat
//! `Ok`, or whose walk fails, is not walked and is kept as that mount's last known state ([`State::seen`]) — the scan
//! still applies the others. A mount can be read only when walked and `Ok` when last seen — one rule for every reader.
//! A read or write inside one that cannot be stats it fresh (one not there takes that state without a ticket), walks
//! it again if it is back, and otherwise refuses [`LIBRARY_UNAVAILABLE`] with `mount` before any change, so a write
//! never touches its disk under the write lock; a relink skips its files. Whole-tree reads answer from the readable
//! mounts and name the others (ADR-0400 decisions 6 and 17).
//!
//! The root generation starts each run at the base the daemon draws (`LibraryValues::root_generation_base`), so a
//! generation kept from an earlier run does not match; generations are compared only for equality (ADR-0381).
//!
//! ## Preset views and edits
//!
//! `edit.rs` holds the `preset.get` view, the preset edits, the draft check and the folder preview. Each edit is one
//! [`LibraryService::change_at`]: link admission, then the read of the file's current bytes, the operation, the write
//! (skipped when nothing changed) and the per-item apply, all under the write lock; the folders an `add` names are
//! stat'ed before it, outside every lock. A preset item carries `rev`, the SHA-256 of the bytes it was made from
//! (ADR-0378), so `preset.get` reads no preset file. The rules are in the `edit.rs` header.
//!
//! ## Agent-preset view and edits
//!
//! `agent_edit.rs` holds the `agent.presets` view and the attachment edits. An edit never mutates the profile
//! registry: inside one [`LibraryService::change_at`] it reads the agent, judges the request against the catalog and
//! writes through the profile-edit port, which the daemon plugs in once ([`LibraryService::plug_profile_edit`] — a
//! weak cell); an empty or dead cell refuses `profile-edit-unavailable` (ADR-0380). The rules are in the
//! `agent_edit.rs` header.
//!
//! ## Relink and create
//!
//! `relink.rs` holds `relink`: inside one [`LibraryService::change_at`] it rewrites each preset file that lists an old
//! path through the edits' per-file pipeline, then the agents once through the profile-edit port (ADR-0335); a
//! per-file refusal skips the file and an IO failure stops the walk. `create.rs` holds `newFolder` · `newFile`: inside
//! one change, the store's create refusing a taken name (never a replace) or a missing folder, the new item or folder
//! entered into the catalog.
//! The rules are in each file's header.
//!
//! ## The trash
//!
//! `trash.rs` holds what a mount's `.trash` holds and how it is touched without following a link (ADR-0409): the trash
//! lock — one for every mount, after the library lock and before the turn mutex — and its waiting count, which a step
//! that may touch `.trash` raises before its ticket ([`LibraryService::change_held_at`], a `Change` hold whose body
//! raises the change number before its agents pass — ADR-0409 M1); the trash-folder check, bundle ids, the bundle record and the
//! bundle-shell pruning; the `.git/info/exclude` line. The store's renames never replace (`store.rs`). The rules are in
//! its header.
//!
//! ## Refusals
//!
//! A body refuses with [`LibraryRefusal`]: a [`RefusalClass`] plus an agent `launch::Reason`, or the profile store's
//! own error passed through. `host.rs` hands it 1:1 to agent as its port refusal, and agent maps the class onto the
//! bus code (ADR-0379); a refusal that applied nothing maps onto a code that does not keep the request id, except the
//! two `INTERNAL` cases of ADR-0379 decision 5 (`profile-edit-unavailable` and a profile-store write failure).
//!
//! ## Commands
//!
//! agent declares the commands and owns their vocabulary; `host.rs` implements its `LibraryCommandHost` port: it
//! parses the spellings with the bodies' own path rules, calls one body per command and converts the answer into
//! agent's declared rows, with no second body call and no read-back (ADR-0384). Bus codes stay in agent. The port
//! carries the `library.*`, `preset.*` and agent-preset commands and the `agent.new {presets}` judgement; the daemon
//! plugs this service into the command table.
// ADR-0311
// ADR-0324
// ADR-0378
// ADR-0379
// ADR-0380
// ADR-0381
// ADR-0382
// ADR-0386
// ADR-0409

pub mod agent_edit;
pub mod backends;
pub mod builtin;
pub mod catalog;
pub mod config;
pub mod create;
pub mod edit;
pub mod excerpt;
pub mod folder;
pub mod format;
mod gate;
pub mod host;
pub mod launch;
pub mod mounts;
pub mod path;
pub mod refs;
pub mod relink;
pub mod rename;
pub mod resolve;
pub mod store;
pub mod trash;
pub mod validate;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::SystemTime;

use engram_dashboard_agent::launch::{
    backend_word, escape_controls, BlockReason, Reason, BUILTIN_UNAVAILABLE,
};
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, StoreError,
};
use engram_dashboard_agent::profile_edit::ProfileEditPort;
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_base::sync::{lock, wait_timeout};
use engram_dashboard_platform::fs::{normalize_folder, path_key, same_path, PathKey};
use serde_json::json;

use crate::backends::Backends;
use crate::builtin::file_of;
use crate::builtin::Builtins;
use crate::catalog::{
    key, preset_refs, within, Catalog, Item, ItemContent, ItemKind, ScanItem, ScanResult,
};
use crate::config::{ConfigFile, ConfigHealth, LibraryConfig, MountPlace, MountSetting};
use crate::folder::{BoundedIo, FolderStatus, LaunchIo, Need, NOT_CHECKED, SPAWN_FAILED};
use crate::format::read::ITEM_PATH_INVALID;
pub use crate::gate::{
    ChangeAnswer, Hold, MountSeen, MountState, MountsNow, Numbers, State, NOT_FOUND,
};
use crate::gate::{Gate, MountChange, Tags};
use crate::mounts::{
    builtin_rest, in_builtin, is_builtin, overlap, overlapping, virtual_path, DiskProbe, Mount,
    MountName, MountNameError, MountPath, Mounts, BUILTIN_MOUNT,
};
use crate::path::{AssetRef, BuiltinId, LibPath, PathError};
use crate::refs::{closure_by, default_slot_count, reference_count, referrers_of};
use crate::resolve::{
    resolve, CatalogLookup, Lookup, MountStates, Resolution, RootState, TagRule, Target,
};
use crate::store::LibraryStore;

/// The largest library file a scan or the list safety net reads. A longer file is a failed read: its item is kept as
/// it was and the scan reports the error.
pub const LIBRARY_READ_CAP: u64 = 16 * 1024 * 1024;

/// The code of a path that lies at or below a link inside the library (`args` = `{path}`) — such a path is never read
/// or merged (U1).
// ADR-0365
// TRD S25 step-7 B8
pub const PATH_THROUGH_LINK: &str = "path-through-link";

/// The code of a catalog read that could not be answered — no scan of the library could be applied (`args` =
/// `{message}`), or its range lies in a mount that cannot be read (`args` = `{mount, message}` — ADR-0400 decision 6)
/// — and of the ④ row, a library item reached in a mount that cannot be used (`args` = `{mount, message}`, one row per
/// mount). A launch leaves that row out for an absent mount and refuses on it for one that exists but cannot be read
/// (ADR-0387 · ADR-0400 decision 19).
// TRD S25 step-7 B8
pub const LIBRARY_UNAVAILABLE: &str = "library-unavailable";

/// The unreadable reason of a folder stat the bounded IO gave no answer for — never expected (`BoundedIo::stat_all`
/// answers every key it is asked); answered instead of aborting the command.
// ADR-0293
// ADR-0386
pub const STAT_UNANSWERED: &str = "the bounded stat gave no answer";

/// The unreadable reason of a folder listing the bounded IO gave no answer for — never expected
/// (`BoundedIo::list_all` answers every key it is asked); answered instead of aborting the launch check.
// ADR-0293
// ADR-0386
pub const LISTING_UNANSWERED: &str = "the bounded listing gave no answer";

/// The code of an edit whose root generation is not the current one (`args` = `{given, current}`).
// ADR-0377
pub const ROOT_GENERATION_STALE: &str = "root-generation-stale";

/// The code of a write into the built-in mount `engram` — it is read-only (`args` = `{mount, path}`).
// ADR-0400 decision 20
pub const MOUNT_READ_ONLY: &str = "mount-read-only";

/// The code of a write whose target is the virtual root or a mount's top — a path of one name, or none (`args` =
/// `{path}`, absent for none): nothing is created at the top, and a mount's top changes only through the mount
/// commands.
// ADR-0400 decision 5
pub const VIRTUAL_ROOT_READ_ONLY: &str = "virtual-root-read-only";

/// The code of an `addMount` or `relocateMount` whose folder cannot be used — it could not be created, its stat did
/// not answer `Ok`, whether it overlaps could not be judged (an ask gave no answer — fail closed), or its walk failed
/// at the folder; nothing published, `library.json` not written (`args` = `{path, why}`).
// ADR-0379
// ADR-0381
// ADR-0400 decision 10
pub const LIBRARY_ROOT_UNUSABLE: &str = "library-root-unusable";

/// The code of a mount command while `library.json` is read-only for the run — newer or unreadable at load (`args` =
/// `{why}`).
// ADR-0376
// ADR-0400 decision 15
pub const LIBRARY_CONFIG_READ_ONLY: &str = "library-config-read-only";

/// The code of a mount command whose `library.json` write was refused or failed after the change was published —
/// published but not saved: the change stays live this run, the file keeps its setting, the next start follows the
/// file that is left, and repeating the same call saves again (`args` = `{why}`, which says so).
// ADR-0379
// ADR-0381
// ADR-0400 decision 14
pub const LIBRARY_CONFIG_WRITE_FAILED: &str = "library-config-write-failed";

/// The code of a mount name the name rules refuse (`args` = `{name, why}`).
// ADR-0400 decision 2
pub const MOUNT_NAME_INVALID: &str = "mount-name-invalid";

/// The code of an `addMount` whose name is registered at another folder, or a `renameMount` whose new name is another
/// mount's (`args` = `{name}` — the registered spelling).
// ADR-0400 decisions 7 and 12
pub const MOUNT_NAME_TAKEN: &str = "mount-name-taken";

/// The code of an `addMount` or `relocateMount` whose folder is another mount's, holds one or lies inside one (`args` =
/// `{path, with, mount}`: `with` = `mount`, `mount` names it). Only mounts are judged.
// ADR-0400 decision 10
pub const MOUNT_OVERLAPS: &str = "mount-overlaps";

/// The code of a mount command naming a mount that is not registered (`args` = `{mount}`).
// ADR-0400 decision 7
pub const MOUNT_NOT_FOUND: &str = "mount-not-found";

/// The code of a `renameMount` whose mount cannot be read — absent, unreadable, held as overlapping another, or not
/// walked — so its own references could not be rewritten (`args` = `{mount, state, why}`, `state` = `missing` |
/// `unreadable`); nothing applied.
// ADR-0400 decision 12
pub const MOUNT_UNUSABLE: &str = "mount-unusable";

/// The code of a `renameMount` that would rewrite files or leave references unread (a referencing file in a mount that
/// cannot be read, or a mount never walked) and was not confirmed — no `yes`, `yes` without the `root_generation` or
/// the `plan_digest` the plan answered, or the plan is no longer the one that digest names (`args` = `{plan,
/// root_generation, plan_digest}` — the plan now); nothing applied.
// ADR-0400 decision 12
pub const MOUNT_RENAME_UNCONFIRMED: &str = "mount-rename-unconfirmed";

const MERGE_JOB: &str = "library-merge";
const BOOT_SCAN_JOB: &str = "library-boot-scan";
const EXCLUDE_JOB: &str = "library-trash-exclude";

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

/// Why a body refused. A refusal applied nothing to the library or the registry — folders a mount command's `create`
/// made before it refused [`LIBRARY_ROOT_UNUSABLE`] do not count as applied (ADR-0379 decision 4) — except
/// [`LIBRARY_CONFIG_WRITE_FAILED`], which comes after the mount change was published and still maps onto a code that
/// does not keep the request id, so a retry saves again (ADR-0381).
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

/// The refusal of a root generation that is not the current one — `Conflict` · [`ROOT_GENERATION_STALE`].
// ADR-0377
fn stale_root_generation(given: Option<u64>, current: u64) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        ROOT_GENERATION_STALE,
        json!({ "given": given, "current": current }),
    )
}

/// What the daemon hands the service.
pub struct LibraryValues {
    pub registry: Arc<ProfileRegistry>,
    /// `AgentBackends` in production.
    pub backends: Arc<dyn Backends>,
    /// `library.json` as `config::load` read it — read only by `new`; after that the service's own copy is live.
    pub config: LibraryConfig,
    /// The place a mount's `"default"` path names this run (ADR-0375) — also `status`'s `suggested_new`.
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
    /// Serializes the mount commands and the first start, each across its change and its save (module header).
    mount_change: Mutex<()>,
    /// The mounts held as overlapping an earlier one at load, each with the `why` it is read as unreadable by — never
    /// walked until relocated or removed (ADR-0400 decision 10). A leaf lock: nothing else is taken while it is held.
    overlapping: Mutex<Vec<(Mount, String)>>,
    /// How far the load-time overlap judgement is ([`LibraryService::judge_at_load`]); `load_judged` wakes its waiters.
    /// A leaf held for no IO: the judgement runs outside it, never under a library lock or a ticket, and is run only
    /// before the mount-change mutex — under it (a mount command's final status) the judgement is already done.
    load_judgement: Mutex<Judgement>,
    load_judged: Condvar,
    /// The profile-edit port, plugged in once by the daemon ([`LibraryService::plug_profile_edit`]); weak, so the
    /// manager, which holds this service as its launch-kit source, makes no cycle.
    // ADR-0380
    profile_edit: OnceLock<Weak<dyn ProfileEditPort>>,
    /// Serializes the `.git/info/exclude` writes (`trash::exclude_trash`) — a leaf, taken by the exclude job alone, holding
    /// no other lock.
    // ADR-0409 M12
    exclude: Arc<Mutex<()>>,
    /// How many mount-change takes found it held — tests observe a waiter.
    #[cfg(test)]
    mount_change_blocked: std::sync::atomic::AtomicUsize,
    /// How many callers found the load judgement running and waited for it — tests observe the waiters.
    #[cfg(test)]
    load_judge_blocked: std::sync::atomic::AtomicUsize,
}

/// How far the load-time overlap judgement of one run is ([`LibraryService::judge_at_load`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Judgement {
    NotStarted,
    Running,
    Done,
}

/// The run of the load-time judgement by one caller: [`JudgementRun::finish`] marks it done; dropped unfinished (a
/// panic), it is marked not started again so the next caller runs it. Either way its waiters are woken.
struct JudgementRun<'s> {
    service: &'s LibraryService,
    done: bool,
}

impl JudgementRun<'_> {
    fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for JudgementRun<'_> {
    fn drop(&mut self) {
        *lock(&self.service.load_judgement) = match self.done {
            true => Judgement::Done,
            false => Judgement::NotStarted,
        };
        self.service.load_judged.notify_all();
    }
}

/// The answer of [`LibraryService::status`].
// ADR-0400 decision 7
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryStatus {
    pub daemon_id: String,
    /// Every mount: the registered ones in list order, then the built-in `engram`.
    pub mounts: Vec<MountRow>,
    pub config: ConfigHealth,
    /// The default place.
    pub suggested_new: PathBuf,
    /// The value an edit may carry ([`LibraryService::change_at`]); every mount change raises it.
    pub root_generation: u64,
}

/// One mount of [`LibraryStatus`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountRow {
    pub name: String,
    /// The folder it maps to — the built-in's: the install's built-in folder, empty when there is none.
    pub folder: PathBuf,
    /// Its folder is the default place — stored as `"default"`.
    pub default: bool,
    /// The built-in mount `engram`.
    pub builtin: bool,
    /// Its folder's stat now (a mount held as overlapping another: unreadable, naming it); the built-in's as read at
    /// start.
    pub state: MountState,
}

/// What a mount command answers besides the status: what points at the mount's name from outside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointing {
    /// References listed in the preset files of the other mounts.
    pub presets: usize,
    /// Agents with an attachment in the mount.
    pub agents: usize,
}

/// The answer of [`LibraryService::add_mount`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountAnswer {
    pub status: LibraryStatus,
    /// What already pointed at the name — it now resolves into the new folder.
    pub pointing: Pointing,
}

/// The answer of [`LibraryService::remove_mount`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveAnswer {
    pub status: LibraryStatus,
    /// What now points at a mount that is not registered — it reads absent.
    pub now_absent: Pointing,
    pub outcome: Removal,
}

/// What a `removeMount` did.
// ADR-0400 decision 14
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    Removed,
    /// The mount was not registered: a repeat of a removal whose save failed — only `library.json` was written.
    SavedOnly,
}

/// The answer of [`LibraryService::first_start`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstStart {
    /// `library.json` was not missing at load, or the step already published this run — nothing done. A `Failed`
    /// step stays due, so a second call in the same run would retry.
    NotDue,
    /// The default place was created, stat'ed and published without a walk as the mount `default_library`, and then
    /// `library.json` written with it at `"default"`; the boot scan walks it ([`LibraryService::start_boot_scan`]).
    // ADR-0382
    Published,
    /// The default mount was published but `library.json` was not written (`why`): live this run, `config: unsaved`;
    /// the next start follows the file that is left (with none left, the first start runs again).
    // ADR-0381
    Unsaved(String),
    /// The default place could not be created, its stat did not answer `Ok`, or a mount was already published
    /// (only a test publishes one before the first start): nothing published or written, the mounts as they were (none
    /// unless already published); the next start tries again.
    Failed(String),
}

/// The answer of [`LibraryService::folders`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldersAnswer {
    /// Every mount's top (the built-in `engram` included), then every folder of the mounts that can be read and the
    /// built-in's virtual folders, by path; a skill folder is marked.
    pub folders: Vec<FolderNode>,
    /// The mounts that cannot be read: a library mount's folders were left out; the built-in mount keeps its virtual
    /// folders.
    // ADR-0400 decision 6
    pub unavailable: Vec<UnavailableMount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderNode {
    pub path: String,
    pub name: String,
    pub skill: bool,
    /// A mount's top: its last known state (no fresh stat); `None` = a folder inside a mount.
    // ADR-0400 decision 4
    pub mount_state: Option<MountState>,
}

/// A mount a whole-tree answer names because its items cannot be read, with the state it was last seen in — a library
/// mount (its items left out) or the built-in mount `engram` with no built-in folder (its rows stay, as stand-ins).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableMount {
    /// The mount's name — [`BUILTIN_MOUNT`] for the built-in, which no [`MountName`] can hold.
    pub mount: String,
    pub state: MountState,
}

/// The answer of [`LibraryService::read`].
#[derive(Clone, Debug, PartialEq)]
pub struct ReadAnswer {
    /// Kind, name, state, description and reference count as `list` shows it.
    pub row: ListRow,
    /// The item on disk in its mount's folder; a built-in's file in the built-in folder.
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
    /// The items directly in the folder, by path — a skill folder is an item (kind = skill), a built-in is an item of
    /// the mount `engram`; with a query, every matching item below it.
    pub rows: Vec<ListRow>,
    /// The plain folders directly in the folder (not skill folders), by path; none with a query.
    pub folders: Vec<FolderRow>,
    /// With a query and no folder: the mounts that cannot be read — a library mount's items were left out; the
    /// built-in mount's rows stay, as stand-ins.
    // ADR-0400 decision 6
    pub unavailable: Vec<UnavailableMount>,
}

/// What [`LibraryService::list_with`] keeps. The default keeps everything.
// ADR-0386
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListFilter {
    /// A case-insensitive substring of a row's name or description, searched in every item below the folder.
    pub query: Option<String>,
    /// The kinds kept, compared with [`ItemKind::listed`]; empty = every kind.
    pub kinds: Vec<ItemKind>,
}

impl ListFilter {
    fn keeps(&self, row: &ListRow, query: Option<&str>) -> bool {
        let kind_kept = self.kinds.is_empty()
            || row
                .kind
                .listed()
                .is_some_and(|kind| self.kinds.contains(&kind));
        let query_kept = query.is_none_or(|query| {
            row.name.to_lowercase().contains(query)
                || row.description.to_lowercase().contains(query)
        });
        kind_kept && query_kept
    }
}

/// One plain subfolder line of a [`ListAnswer`].
#[derive(Clone, Debug, PartialEq)]
pub struct FolderRow {
    pub path: String,
    /// Its last name.
    pub name: String,
    /// A mount's top (a row of the virtual root): its last known state (no fresh stat); `None` = a folder inside a
    /// mount.
    // ADR-0400 decision 4
    pub mount_state: Option<MountState>,
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
    /// Links below a mount's folder that were not entered.
    // ADR-0365
    pub skipped_links: Vec<LibPath>,
    /// Where the walk or a read failed (a mount's top for its folder), with the error; those items are kept as they
    /// were. When nothing could be scanned (a mount's folder unreadable, a folder in no registered mount, or a
    /// refresh's own scan of a folder with a dot or temp name), the one entry is `(None, why)`.
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
    /// `io` bounds every folder operation outside the library and spawns the service's jobs (short merge jobs and the boot scan). The built-ins
    /// are read here, once.
    pub fn new(values: LibraryValues, store: Arc<dyn LibraryStore>, io: BoundedIo) -> Self {
        let mounts = ConfigFile::initial_mounts(&values.config, &values.default_root);
        let gate = Arc::new(Gate::new(mounts, values.root_generation_base));
        let builtins = Builtins::new(values.builtin_dir.as_deref(), &*values.backends);
        let config = ConfigFile::new(&values.config);
        LibraryService {
            values,
            store,
            io,
            gate,
            builtins,
            config,
            mount_change: Mutex::new(()),
            overlapping: Mutex::new(Vec::new()),
            load_judgement: Mutex::new(Judgement::NotStarted),
            load_judged: Condvar::new(),
            profile_edit: OnceLock::new(),
            exclude: Arc::new(Mutex::new(())),
            #[cfg(test)]
            mount_change_blocked: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            load_judge_blocked: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Plugs in the profile-edit port the agent-preset bodies write through (`agent_edit.rs`) — the daemon's manager,
    /// right after it is built and before any command can reach the service. Set once: a second plug is ignored and
    /// logged. Until it is plugged, or once its manager is gone, those bodies refuse `profile-edit-unavailable`.
    // ADR-0380
    pub fn plug_profile_edit(&self, port: Weak<dyn ProfileEditPort>) {
        if self.profile_edit.set(port).is_err() {
            tracing::warn!("profile-edit port already plugged; second plug ignored");
        }
    }

    /// The built-in priming text read when the service was made — what a launch whose plan includes the priming merges
    /// in first. Empty for its stand-in; then no plan includes it, and the launch check tells `builtin-unavailable` when
    /// the preset reaches `engram/priming` (a preset that does not list it includes no priming and tells nothing).
    pub fn builtin_priming_text(&self) -> &str {
        self.builtins.priming_text()
    }

    /// One library preset on its own (the edit area — no agent: everything is shown, no format verdict), after the
    /// first-scan entrance, over the catalog under one read lock; its folder entries are stat'ed afterwards, outside
    /// the lock, on one read-computation budget. The folders of the mounts it reaches are stat'ed: one that is not there
    /// is a skipped ④ row (`missing` / `not a folder`); one that is there but cannot be read blocks with a ④ row
    /// ([`read_states`]).
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

    /// What each agent gets (tree ⛔; `agent.presets` resolves the one profile it read the same way): one first-scan
    /// entrance, the stats of the mounts the attached library presets reach ([`reached_mounts`]), one read lock for all
    /// of them, then the folder
    /// stats — all on one read-computation budget, each distinct folder stat'ed once; then, with no lock, a link probe
    /// for each ① `not-found` library row. `None` = no such agent, or its backend has no settings check (it takes no
    /// preset — B13).
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

    /// One resolution per want, in order. Locks: the first-scan entrance, a brief read lock for the mounts the attached
    /// library presets reach, their stats with no lock, one read lock that judges the mounts and resolves every want,
    /// the folder stats with no lock, then the link probes of `mark_links` with no lock. The items of a mount that
    /// cannot be used are not looked up, so they are neither stat'ed nor probed.
    fn resolve_wants(&self, wants: &[&Want]) -> Vec<Resolution> {
        self.resolve_over(wants, true, None, |_, _, _| ()).0
    }

    /// [`LibraryService::resolve_wants`] with `draft` standing in for the catalog item at its path (`preset.check`)
    /// and `extra` run over the state, the same lookup and the same mount states inside the read lock that resolves.
    /// `library` = the computation reads library items; `false` skips the first-scan entrance and the mount stats and
    /// takes every mount as usable (only built-ins and `draft` are looked up).
    pub(crate) fn resolve_over<R>(
        &self,
        wants: &[&Want],
        library: bool,
        draft: Option<&edit::Draft>,
        extra: impl FnOnce(&State, &dyn Lookup, &MountStates) -> R,
    ) -> (Vec<Resolution>, R) {
        let joined = if library {
            self.first_scan_join()
        } else {
            Ok(())
        };
        let backends = &*self.values.backends;
        let attached: Vec<(TagRule, &AssetRef)> = wants
            .iter()
            .flat_map(|want| {
                let rule = TagRule::of(backends, want.backend.as_deref(), want.mode);
                want.presets.iter().map(move |preset| (rule, preset))
            })
            .collect();
        let reached = match library {
            true => self
                .gate
                .with_read(|state| reached_mounts(&state.catalog, &state.mounts, &attached)),
            false => Vec::new(),
        };
        let io = self.io.begin_read();
        let stats = self.stat_reached(&io, &reached);
        let (mounts, mut resolved, extra) = self.gate.with_read(|state| {
            // The states are judged over the mounts this lock resolves with: a mount renamed or removed after the
            //   stats is not judged under a name the lookup no longer knows.
            let states = match library {
                true => read_states(&state.mounts, |name| state.seen(name), &stats, &joined),
                false => MountStates::new(&state.mounts, false),
            };
            let catalog = CatalogLookup {
                catalog: &state.catalog,
                mounts: &state.mounts,
                builtins: &self.builtins,
            };
            let overlaid = draft.map(|draft| draft.over(&catalog));
            let lookup: &dyn Lookup = match &overlaid {
                Some(overlaid) => overlaid,
                None => &catalog,
            };
            let resolved: Vec<Resolution> = wants
                .iter()
                .map(|want| Target {
                    backend: want.backend.clone(),
                    mode: want.mode,
                    default_preset: want.default_preset,
                    presets: want.presets.clone(),
                    unparsed: want.unparsed.clone(),
                    mounts: states.clone(),
                })
                .map(|target| resolve(&target, lookup, backends))
                .collect();
            let extra = extra(state, lookup, &states);
            (state.mounts.clone(), resolved, extra)
        });
        let folders: Vec<(PathBuf, Need)> = resolved
            .iter()
            .flat_map(Resolution::folder_paths)
            .map(|path| (path, Need::Required))
            .collect();
        let statuses = self.io.stat_all(&io, &folders);
        for resolution in &mut resolved {
            resolution.attach_folder_status(&statuses);
            self.mark_links(&mounts, resolution);
        }
        (resolved, extra)
    }

    /// The folder of each mount of `reached` stat'ed as a required ask on `io` and judged with that mount's first-scan
    /// answer ([`LibraryService::stat_named`]), by [`mount_key`] — what [`read_states`] judges them by.
    // ADR-0387
    pub(crate) fn stat_reached(&self, io: &LaunchIo, reached: &[Mount]) -> ReadStats {
        if reached.is_empty() {
            return ReadStats::new();
        }
        let statuses = self.stat_named(io, reached);
        reached.iter().map(mount_key).zip(statuses).collect()
    }

    /// [`LibraryService::stat_mount_list`] of `mounts`, each status then judged with that mount's first-scan answer.
    /// A mount that stats `Ok` but whose items cannot be read (never walked this run, or not `Ok` when last seen) is
    /// walked again first — the next reader whose range touches it (ADR-0400 decision 17). One still not readable after
    /// that answers its last known state, the rule `list` and `read` use: missing (it vanished during the walk) is
    /// absent; otherwise unreadable, as [`RootState::of`] judges a root whose first scan failed (ADR-0387 per mount). A
    /// mount that does not stat `Ok` is not walked again — its stat is the answer. A mount no longer registered at its
    /// folder keeps its stat. Each mount is judged as itself, so two mounts on one folder keep their own answers.
    pub(crate) fn stat_named(&self, io: &LaunchIo, mounts: &[Mount]) -> Vec<FolderStatus> {
        let statuses = self.stat_mount_list(io, mounts);
        let registered = |state: &State, mount: &Mount| {
            state
                .mounts
                .get(mount.name.as_str())
                .is_some_and(|live| same_path(&live.folder, &mount.folder))
        };
        let stale: Vec<MountName> = self.gate.with_read(|state| {
            mounts
                .iter()
                .zip(&statuses)
                .filter(|(_, status)| **status == FolderStatus::Ok)
                .filter(|(mount, _)| registered(state, mount))
                .filter(|(mount, _)| !state.seen(&mount.name).usable())
                .map(|(mount, _)| mount.name.clone())
                .collect()
        });
        if !stale.is_empty() {
            self.rewalk_unusable(&stale);
        }
        self.gate.with_read(|state| {
            mounts
                .iter()
                .zip(statuses)
                .map(|(mount, status)| {
                    let seen = registered(state, mount).then(|| state.seen(&mount.name));
                    match (status, seen) {
                        // One rule with `list` and `read`: readable only when walked and `Ok` when last seen.
                        (FolderStatus::Ok, Some(seen)) if !seen.usable() => match seen.shown() {
                            // Gone during the walk again: absent, skipped — not a block (ADR-0387).
                            MountState::Missing { .. } => FolderStatus::Missing,
                            MountState::Unreadable { why } => FolderStatus::Unreadable(why),
                            MountState::Ok => FolderStatus::Ok,
                        },
                        (status, _) => status,
                    }
                })
                .collect()
        })
    }

    /// Each folder of `folders` stat'ed as a required ask on `io`, all in one batch (with the share probe of
    /// [`BoundedIo::stat_all`]) — one wait for all, so a mount that does not answer cannot spend the budget of the
    /// others; answered in the order of `folders`. [`STAT_UNANSWERED`] if the bounded IO broke its every-key contract.
    // ADR-0360
    pub(crate) fn stat_mounts(&self, io: &LaunchIo, folders: &[PathBuf]) -> Vec<FolderStatus> {
        let asks: Vec<(PathBuf, Need)> = folders
            .iter()
            .map(|folder| (folder.clone(), Need::Required))
            .collect();
        let statuses = self.io.stat_all(io, &asks);
        folders
            .iter()
            .map(|folder| {
                statuses.get(&path_key(folder)).cloned().unwrap_or_else(|| {
                    tracing::error!(
                        path = %escape_controls(&folder.to_string_lossy()),
                        "the bounded stat gave no answer for the path it was asked"
                    );
                    FolderStatus::Unreadable(STAT_UNANSWERED.to_owned())
                })
            })
            .collect()
    }

    /// [`LibraryService::stat_mounts`] of the folders of `mounts` after the load-time overlap judgement
    /// ([`LibraryService::judge_at_load`]), a mount held as overlapping another answering `Unreadable` with that `why`
    /// where its folder stats `Ok` — the one stat every read entrance, launch and `status` takes of a mount (a scan's
    /// is [`LibraryService::scan_stats`]), so a held mount is never walked or read (ADR-0400 decision 10). A new caller
    /// that stats a mount's folder goes through one of the two — except a rename's in-hold stat of a mount whose file
    /// read as not found (`rename.rs` `mount_gone`), which runs after the judgement on a mount known not held.
    pub(crate) fn stat_mount_list(&self, io: &LaunchIo, mounts: &[Mount]) -> Vec<FolderStatus> {
        self.stat_held(io, mounts, |status| status)
    }

    /// The load-time judgement, then the folders of `mounts` stat'ed on `io`, each status passed through `adjust`, and
    /// then a held mount's `Ok` turned into `Unreadable` with its hold's `why` — after `adjust`, so nothing `adjust`
    /// reads as `Ok` walks a held mount.
    // ADR-0400 decision 10
    fn stat_held(
        &self,
        io: &LaunchIo,
        mounts: &[Mount],
        adjust: impl Fn(FolderStatus) -> FolderStatus,
    ) -> Vec<FolderStatus> {
        if !self.judge_at_load(io) {
            // Its budget is spent waiting for the judgement: no ask, and no mount taken as `Ok`.
            return mounts
                .iter()
                .map(|_| FolderStatus::Unreadable(NOT_CHECKED.to_owned()))
                .collect();
        }
        let folders: Vec<PathBuf> = mounts.iter().map(|mount| mount.folder.clone()).collect();
        self.stat_mounts(io, &folders)
            .into_iter()
            .map(adjust)
            .zip(mounts)
            .map(|(status, mount)| match (status, self.held_overlap(mount)) {
                (FolderStatus::Ok, Some(why)) => FolderStatus::Unreadable(why),
                (status, _) => status,
            })
            .collect()
    }

    /// The root (or any folder — `LaunchKitSource::stat_folder`) stat'ed as a required ask on `io` (with the share probe
    /// of [`BoundedIo::stat_all`]); [`STAT_UNANSWERED`] if the bounded IO broke its every-key contract.
    fn stat_root(&self, io: &LaunchIo, root: &Path) -> FolderStatus {
        // ADR-0293: `stat_all` answers every key it is asked, but a broken answer is a refusal here, never an abort.
        self.io
            .stat_all(io, &[(root.to_path_buf(), Need::Required)])
            .remove(&path_key(root))
            .unwrap_or_else(|| {
                tracing::error!(
                    path = %escape_controls(&root.to_string_lossy()),
                    "the bounded stat gave no answer for the path it was asked"
                );
                FolderStatus::Unreadable(STAT_UNANSWERED.to_owned())
            })
    }

    /// [`LibraryService::stat_root`] as the string answer of a folder stat: `Err(why)` = anything but `Ok` — `missing`,
    /// `not a folder`, or the unreadable reason ([`FolderStatus::why`]).
    pub(crate) fn stat_words(&self, io: &LaunchIo, folder: &Path) -> Result<(), String> {
        match self.stat_root(io, folder) {
            FolderStatus::Ok => Ok(()),
            other => Err(other.why()),
        }
    }

    /// Finishes a resolution built under a library lock, outside every lock: its folders stat'ed on a fresh read budget,
    /// then the link probes of its `not-found` rows in `mounts`.
    pub(crate) fn settle_resolution(&self, resolution: &mut Resolution, mounts: &Mounts) {
        let asks: Vec<(PathBuf, Need)> = resolution
            .folder_paths()
            .into_iter()
            .map(|path| (path, Need::Required))
            .collect();
        let statuses = self.io.stat_all(&self.io.begin_read(), &asks);
        resolution.attach_folder_status(&statuses);
        self.mark_links(mounts, resolution);
    }

    /// Turns each ① `not-found` row of a library reference that lies at or below a link inside the library into ②
    /// [`PATH_THROUGH_LINK`] (R11), or into ② [`launch::LIBRARY_READ_FAILED`] when the probe fails — the rule of
    /// [`link_verdict`]. Asks the store's `first_link` per row in its mount's folder (a row of a mount not in `mounts`,
    /// of a mount's top, or of a mount the resolution found absent is left as it is) — call it outside every lock.
    // ADR-0387
    // ADR-0400 decision 19: an absent mount's folder is not touched.
    pub(crate) fn mark_links(&self, mounts: &Mounts, resolution: &mut Resolution) {
        let absent: Vec<PathKey> = resolution
            .absent_mounts
            .iter()
            .map(|(name, _)| path_key(Path::new(name)))
            .collect();
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
            let first = path.as_str().split('/').next().unwrap_or_default();
            if absent.contains(&path_key(Path::new(first))) {
                continue;
            }
            let Some(probe) = self.first_link(mounts, &path) else {
                continue;
            };
            if let Some(verdict) = link_verdict(probe, item, reason.line) {
                *problem = BlockReason::Broken {
                    item: item.clone(),
                    via: via.clone(),
                    reason: verdict,
                };
            }
        }
    }

    /// The store's link probe of `path` in its mount's folder (`LibraryStore::first_link`); `None` = nothing to probe —
    /// its mount is not in `mounts`, or it names a mount's top. A store call — run it outside every lock.
    pub(crate) fn first_link(
        &self,
        mounts: &Mounts,
        path: &LibPath,
    ) -> Option<io::Result<Option<MountPath>>> {
        let (mount, Some(rest)) = mounts.locate(path)? else {
            return None;
        };
        Some(self.store.first_link(&mount.folder, &rest))
    }

    /// What agent `id` attaches, read from the registry (outside every library lock).
    fn agent_want(&self, id: AgentId) -> Option<Want> {
        self.want_of(&self.values.registry.get(id)?)
    }

    /// The tag rule `profile`'s resolution follows references by ([`TagRule::of`] over what
    /// [`LibraryService::want_of`] reads).
    pub(crate) fn tag_rule(&self, profile: &AgentProfile) -> TagRule<'_> {
        TagRule::of(
            &*self.values.backends,
            Some(&backend_word(&profile.command)),
            Some(output_format(&profile.command)),
        )
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

    /// The items and plain subfolders directly in `folder` (`None` = the virtual root, whose subfolders are the mounts'
    /// tops), after the first-scan entrance, under
    /// one read lock — no scan once the first scan is raised. `Err` = the first scan could not be applied
    /// ([`LIBRARY_UNAVAILABLE`] — ADR-0324 decision 6: that failure is the answer). A `folder` the catalog does not
    /// know is checked on disk alone and merged (the safety net — a late merge, ADR-0324 decision 5); one at or below
    /// a link answers a single [`PATH_THROUGH_LINK`] row; one with a dot or temp name answers no rows. A folder the
    /// catalog does not know answers its items only — its subfolder lines appear after a refresh.
    pub fn list(&self, folder: Option<&LibPath>) -> Result<ListAnswer, Reason> {
        self.list_with(folder, &ListFilter::default())
    }

    /// [`LibraryService::list`] through `filter`: with a query, the matching items below `folder` and no folder
    /// lines; rows outside `filter.kinds` dropped. A folder at or below a link still answers its one
    /// [`PATH_THROUGH_LINK`] row, unfiltered — it describes the folder asked for, not an item. A folder in the built-in
    /// mount answers from the built-ins, with no first-scan entrance ([`LibraryService::builtin_answer`]).
    // ADR-0386
    // ADR-0400 decision 20
    pub fn list_with(
        &self,
        folder: Option<&LibPath>,
        filter: &ListFilter,
    ) -> Result<ListAnswer, Reason> {
        if let Some(folder) = folder.filter(|folder| in_builtin(folder)) {
            return Ok(self.builtin_answer(folder, filter));
        }
        self.first_scan_join().map_err(|why| Reason {
            code: LIBRARY_UNAVAILABLE.to_owned(),
            args: json!({ "message": why }),
            line: None,
        })?;
        // ADR-0400 decision 6: a folder inside one mount that cannot be read refuses, naming it.
        if let Some((name, state)) = folder.and_then(|folder| self.unusable_mount(folder)) {
            return Err(mount_unavailable(&name, &state));
        }
        // ADR-0324 decision 5: the numbers are noted in the read lock that reads the mounts the disk read uses, so a
        //   mount change published after it moves the root generation and discards the merge.
        let known = self.gate.with_read(|state| match folder {
            Some(path) if !is_known(&state.catalog, path) => {
                Err((path, state.mounts.clone(), self.gate.numbers()))
            }
            _ => Ok(self.answer(state, folder, filter)),
        });
        let (path, mounts, noted) = match known {
            Ok(answer) => return Ok(answer),
            Err(unknown) => unknown,
        };
        match self.observe(&mounts, path) {
            Observed::ThroughLink => {
                return Ok(ListAnswer {
                    folder: folder.cloned(),
                    rows: vec![through_link_row(path)],
                    folders: Vec::new(),
                    unavailable: Vec::new(),
                })
            }
            Observed::Items(items) => {
                self.gate.late_merge(noted, |catalog, may_insert| {
                    catalog.apply_items(items, may_insert)
                });
            }
            Observed::Refused => {}
        }
        Ok(self
            .gate
            .with_read(|state| self.answer(state, folder, filter)))
    }

    /// Re-scans `folder` (`None` = the whole library) on its own ticket, or joins a covering scan queued and not yet
    /// walking (ADR-0324 decision 7). Before the first scan is raised a folder refresh passes through a full scan
    /// (decision 6): it joins a queued one or runs its own and takes its share; if a full scan is already walking, it
    /// waits for it and then scans its folder like any refresh — or, when what it waited for failed, goes through a full
    /// scan once more. A folder in the built-in mount is not scanned — its files
    /// are read once at start — and answers that as its one error, with nothing walked (ADR-0400 decision 20).
    pub fn refresh(&self, folder: Option<&LibPath>) -> RefreshReport {
        if folder.is_some_and(in_builtin) {
            return failed(format!(
                "the built-in mount {BUILTIN_MOUNT} is not scanned: its files are read once at start"
            ));
        }
        match self.gate.refresh(
            folder,
            &|mounts| self.scan_stats(mounts),
            &|mount, under, tags| self.build_scan(mount, under, tags),
        ) {
            Ok(report) => report.share(folder),
            Err(why) => failed(why),
        }
    }

    /// The first-scan entrance every method that reads the catalog passes first, before any ticket or lock (ADR-0324
    /// decision 6). `Err` = no full scan could be applied, with the reason (only a scan that ended unanswered); the
    /// catalog then holds only what late merges put in. A mount the scan could not walk is that mount's state
    /// ([`State::seen`]), not this answer (ADR-0400 decision 17). The launch check does not pass it.
    pub fn first_scan_join(&self) -> Result<(), String> {
        self.gate
            .first_scan_join(&|mounts| self.scan_stats(mounts), &|mount, under, tags| {
                self.build_scan(mount, under, tags)
            })
    }

    /// The stat a scan takes before its ticket: the load-time judgement ([`LibraryService::judge_at_load`]) on a read
    /// budget of its own, then the folders of `mounts` in one bounded batch, with the share probe
    /// ([`LibraryService::stat_mounts`]), on a fresh read budget the judgement charged nothing to — so every answer is
    /// the mount's own, a timeout included, and is its state; a held mount answers its hold even when its stat worker
    /// could not be started ([`LibraryService::stat_held`]). When the judgement is still running once its budget is
    /// spent, no mount is stated (`None` for each): the scan records nothing, walks nothing and does not raise the first
    /// scan on them, and its caller goes round to a fresh scan. That never spins: each such answer is a wait of one
    /// whole budget, and the rounds end once the judgement does (a done judgement stays done and states every mount;
    /// its IO is bounded by one budget, its wait for the library read lock is not). Cost: the first scan that
    /// runs or waits for the judgement spends up to one budget on it before its own stat, once per run.
    // ADR-0400 decisions 10 and 17
    // ADR-0402 decisions 4 and 5
    fn scan_stats(&self, mounts: &[Mount]) -> Vec<Option<FolderStatus>> {
        if !self.judge_at_load(&self.io.begin_read()) {
            return mounts.iter().map(|_| None).collect();
        }
        self.stat_held(&self.io.begin_read(), mounts, |status| match status {
            // ADR-0382 decision 4: no thread could be started for the stat — no word on the mount; the walk on the
            //   caller's thread decides, as the first reader's scan did before stats were taken.
            FolderStatus::Unreadable(why) if why == SPAWN_FAILED => FolderStatus::Ok,
            status => status,
        })
        .into_iter()
        .map(Some)
        .collect()
    }

    /// Walks each mount of `names` again whose items cannot be read — never walked this run, or not `Ok` when last
    /// seen (ADR-0400 decision 17: the next reader whose range touches it). Each is stat'ed fresh first, before any
    /// ticket; one that does not stat `Ok` takes that state without a ticket (nothing queues behind a slow walk), and
    /// only one that does is walked, by a refresh of its top. A mount still not usable after it stays so; the caller
    /// answers from [`State::seen`].
    pub(crate) fn rewalk_unusable(&self, names: &[MountName]) {
        let stale: Vec<Mount> = self.gate.with_read(|state| {
            names
                .iter()
                .filter_map(|name| state.mounts.get(name.as_str()))
                .filter(|mount| !state.seen(&mount.name).usable())
                .cloned()
                .collect()
        });
        if stale.is_empty() {
            return;
        }
        for mount in self.gate.observe(stale, &|mounts| self.scan_stats(mounts)) {
            let top = virtual_path(&mount.name, None);
            let report = self.refresh(Some(&top));
            if !report.errors.is_empty() {
                tracing::debug!(
                    mount = top.as_str(),
                    "library mount walked again for a reader; still not usable"
                );
            }
        }
    }

    /// The mount `path` lies in when that mount cannot be read after [`LibraryService::rewalk_unusable`]: its name and
    /// last known state. `None` = usable, or `path` is in no registered mount (the caller answers not found).
    fn unusable_mount(&self, path: &LibPath) -> Option<(MountName, MountState)> {
        let name = self.gate.with_read(|state| {
            state
                .mounts
                .locate(path)
                .map(|(mount, _)| mount.name.clone())
        })?;
        self.rewalk_unusable(std::slice::from_ref(&name));
        let seen = self.gate.with_read(|state| state.seen(&name));
        (!seen.usable()).then(|| (name, seen.shown()))
    }

    /// Refuses `Unavailable` · [`LIBRARY_UNAVAILABLE`] `{mount, message}` when `path` lies in a mount that cannot be read
    /// after [`LibraryService::unusable_mount`] walked it again — before any ticket, lock or disk IO of the caller's,
    /// so a read or write into a mount that is not there neither answers from a stale catalog nor touches its disk under
    /// the write lock. A path in no registered mount passes (the caller answers not found).
    // ADR-0400 decision 6
    pub(crate) fn refuse_unusable_mount(&self, path: &LibPath) -> Result<(), LibraryRefusal> {
        match self.unusable_mount(path) {
            Some((name, state)) => Err(LibraryRefusal::Refused {
                class: RefusalClass::Unavailable,
                reason: mount_unavailable(&name, &state),
            }),
            None => Ok(()),
        }
    }

    /// `f` over the catalog and the mounts under one read lock. `f` may read the profile registry and call pure backend
    /// port answers (no IO, no lock — TRD S25 step-7 §3-8 step 4) and nothing else outward; it must not call back into
    /// the service.
    pub fn with_read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.gate.with_read(f)
    }

    /// The numbers to note before a disk read whose result will be merged late ([`LibraryService::late_merge`]) — and
    /// before reading the mounts that disk read uses, or inside the same read lock ([`LibraryService::with_read`]):
    /// noted after the mounts were read, they may already include a mount change the read does not see.
    pub fn numbers(&self) -> Numbers {
        self.gate.numbers()
    }

    /// The change number now (`Numbers.change`) — read under the turn mutex alone, a leaf: what `agent.new {presets}`
    /// compares inside the registry's insert with the number its judgement was made at ([`NewPresets::change`] —
    /// ADR-0400 decision 18). It takes no library lock, so it may be asked holding the registry lock.
    ///
    /// [`NewPresets::change`]: crate::agent_edit::NewPresets::change
    // ADR-0400 decision 18
    pub fn change_number(&self) -> u64 {
        self.gate.numbers().change
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
            tracing::warn!("library-merge spawn failed; merging on the calling thread: {e}");
            merge_from(&self.gate, noted, &slot);
        }
    }

    /// A change inside Engram (TRD S25 step-7 §6 writer 5 — step 9a and 9b edits): its own ticket, then `f` under the
    /// write lock with the catalog, the mounts and the sequence; `f` writes per item and returns the touched keys; the
    /// change generation is raised before the lock is released. A caller that reads the catalog passes the first-scan
    /// entrance before this call. The change's disk effect must be finished inside `f` (or before the call): a reader
    /// that notes the numbers after the raise assumes it sees the new disk.
    pub fn change<R>(
        &self,
        f: impl FnOnce(&mut Catalog, &MountsNow, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> R {
        self.gate.change(f)
    }

    /// The stat a folder must pass before it is published as a mount (the first start, add, relocate): one stat on `io`,
    /// outside the gate. `Err(why)` = it did not answer `Ok`.
    fn stat_new_mount(&self, io: &LaunchIo, folder: &Path) -> Result<(), String> {
        self.stat_words(io, folder)
            .map_err(|why| format!("the folder cannot be used as a library mount: {why}"))
    }

    /// [`LibraryService::change`] for an edit that carries the root generation it read from `status` (`None` = the
    /// current mounts). The generation is compared inside the change's own write-lock take, so an edit queued behind a
    /// mount change sees it: a stale one refuses `Conflict` · [`ROOT_GENERATION_STALE`] before `f` runs, nothing
    /// applied. A body that refuses (`Err`) and applied no catalog item moves no generation ([`ChangeAnswer`]).
    // ADR-0377
    pub fn change_at<R: ChangeAnswer>(
        &self,
        root_generation: Option<u64>,
        f: impl FnOnce(&mut Catalog, &MountsNow, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> Result<R, LibraryRefusal> {
        self.gate
            .change_checked(root_generation, f)
            .map_err(|current| stale_root_generation(root_generation, current))
    }

    /// A step-9b hold (ADR-0409 M1): [`LibraryService::change_at`] whose body gets a [`Hold`] — the sequence, a `raise` of
    /// the change number to call before its agents pass (ADR-0400 decisions 18 and 25), and, when `trash`, the trash
    /// lock. `trash` = the step may touch `.trash` (every delete, restore and copy, an overwriting move, a cross-volume
    /// move): the trash waiting count is raised just before the ticket — so call this after any volume probe or copy
    /// — and the call first waits, holding no lock, until no bundle is being emptied; the count is lowered when
    /// this returns, a refusal or a panic included (ADR-0409 decision 14). Without `trash` the body's
    /// [`Hold::take_trash`] answers `Err` at once ([`trash::TRASH_NOT_RESERVED`]); with it the body takes the lock once.
    // ADR-0377
    // ADR-0409 decision 14
    pub fn change_held_at<R: ChangeAnswer>(
        &self,
        root_generation: Option<u64>,
        trash: bool,
        f: impl FnOnce(&mut Catalog, &MountsNow, &Hold) -> (R, Vec<PathKey>),
    ) -> Result<R, LibraryRefusal> {
        self.gate
            .change_held(root_generation, trash, f)
            .map_err(|current| stale_root_generation(root_generation, current))
    }

    /// Hands the `.git/info/exclude` line for the `.trash` of the mount at `mount_folder` (`trash::exclude_trash`) to a
    /// short-lived job, so the step that created a `.trash` never waits on that disk — call it after the hold, holding no
    /// lock. A skip is logged; nothing fails. A job that cannot be started is logged and the line is not written this
    /// time (the next bundle made there checks it again).
    // ADR-0400 decision 24
    pub fn exclude_trash(&self, mount_folder: PathBuf) {
        let serial = Arc::clone(&self.exclude);
        let job = Box::new(move || match trash::exclude_trash(&mount_folder, &serial) {
            trash::Excluded::Added { file } => {
                tracing::info!(
                    file = %escape_controls(&file.to_string_lossy()),
                    "library trash: exclude line added"
                );
            }
            trash::Excluded::Present { .. } | trash::Excluded::NoRepository => {}
            trash::Excluded::Skipped { why } => tracing::warn!(
                folder = %escape_controls(&mount_folder.to_string_lossy()),
                "library trash: exclude line skipped: {}",
                escape_controls(&why)
            ),
        });
        if let Err(e) = self.io.spawner().spawn(EXCLUDE_JOB, job) {
            tracing::warn!("library-trash-exclude spawn failed; exclude line not written: {e}");
        }
    }

    /// Every mount now — the registered ones in list order, then the built-in `engram` — with `library.json`'s state
    /// and the root generation. The registered mounts' folders are stat'ed in one batch on one read budget outside every
    /// lock (a mount held as overlapping answers that); they and the root generation are read in one read lock.
    // ADR-0400 decision 7
    pub fn status(&self) -> LibraryStatus {
        let (mounts, root_generation) = self.gate.with_read(|state| {
            let mounts: Vec<Mount> = state.mounts.iter().cloned().collect();
            (mounts, self.gate.numbers().root)
        });
        let statuses = self.stat_mount_list(&self.io.begin_read(), &mounts);
        let mut rows: Vec<MountRow> = mounts
            .into_iter()
            .zip(statuses)
            .map(|(mount, status)| MountRow {
                name: mount.name.as_str().to_owned(),
                default: self.is_default_place(&mount.folder),
                folder: mount.folder,
                builtin: false,
                state: MountState::of(&status),
            })
            .collect();
        rows.push(MountRow {
            name: BUILTIN_MOUNT.to_owned(),
            folder: self.values.builtin_dir.clone().unwrap_or_default(),
            default: false,
            builtin: true,
            state: self.builtin_mount_state(),
        });
        let (_, config) = self.config.view();
        LibraryStatus {
            daemon_id: self.values.daemon_id.clone(),
            mounts: rows,
            config,
            suggested_new: self.values.default_root.clone(),
            root_generation,
        }
    }

    /// Registers the folder `path` as the mount `name` (ADR-0400 decision 9), creating it first when `create`, and
    /// records it; answers the status after the change and what already points at `name`. In order: the name checked,
    /// the path normalized (it must be absolute), the mount-change mutex taken, a read-only `library.json` refused, the
    /// name looked up — the same name already at the same folder is a repeat (ADR-0400 decision 14): nothing changes
    /// but the save —, then on one read budget the overlap judgement against every mount ([`mounts::overlap`]), the
    /// folder created and its stat ([`LibraryService::judge_new_folder`] has the order), then the mount walked and
    /// published under a `Root` ticket ([`Gate::change_mount`]) and — only after that — `library.json` written with
    /// every mount.
    ///
    /// Refusals: a name the rules refuse `InvalidArgument` · [`MOUNT_NAME_INVALID`]; a relative path `InvalidArgument`
    /// · `item-path-invalid`; a read-only `library.json` `Unavailable` · [`LIBRARY_CONFIG_READ_ONLY`] before anything
    /// is created; a name registered at another folder `Conflict` · [`MOUNT_NAME_TAKEN`]; a folder overlapping a mount
    /// `Conflict` · [`MOUNT_OVERLAPS`] — nothing created; a folder whose overlap cannot be judged (an ask gave no
    /// answer), that cannot be created, does not stat as a folder or whose walk fails `Unavailable` ·
    /// [`LIBRARY_ROOT_UNUSABLE`] — nothing published or written, folders `create` made before the stat or the walk
    /// refused stay; a save refused or failed
    /// after the publish `Unavailable` · [`LIBRARY_CONFIG_WRITE_FAILED`] — the mount stays live, `status` says
    /// `unsaved`, and repeating the call saves again; the load-time overlap judgement still running when the command's
    /// read budget is spent `Unavailable` · [`LIBRARY_UNAVAILABLE`] before anything
    /// ([`LibraryService::take_mount_change`]).
    // ADR-0381
    // ADR-0400 decisions 9, 10, 14 and 15
    pub fn add_mount(
        &self,
        name: &str,
        path: &Path,
        create: bool,
    ) -> Result<MountAnswer, LibraryRefusal> {
        let name = MountName::parse(name).map_err(|why| mount_name_invalid(name, &why))?;
        let folder = absolute_folder(path)?;
        // ADR-0360: the load-time judgement and the folder's own checks share one read budget.
        let read = self.io.begin_read();
        let _serial = self.take_mount_change(&read)?;
        self.refuse_read_only_config()?;
        let live = self
            .gate
            .with_read(|state| state.mounts.get(name.as_str()).cloned());
        match live {
            Some(live) if same_folder(&live.folder, &folder) => {
                self.save_after(&name, &live.folder, "added")?;
            }
            Some(live) => return Err(mount_name_taken(&live.name)),
            None => {
                self.judge_new_folder(&read, &folder, None, create)?;
                let mount = Mount {
                    name: name.clone(),
                    folder: folder.clone(),
                };
                self.gate
                    .change_mount(MountChange::Add(mount), |mount| {
                        self.build_scan(mount, None, &Tags::new())
                    })
                    .map_err(|why| root_unusable(&folder, why))?;
                tracing::info!(
                    mount = name.as_str(),
                    path = %escape_controls(&folder.to_string_lossy()),
                    "library mount added"
                );
                self.save_after(&name, &folder, "added")?;
            }
        }
        Ok(MountAnswer {
            pointing: self.pointing_at(&name),
            status: self.status(),
        })
    }

    /// Drops the registration of the mount `mount` (ADR-0400 decision 11): its range leaves the catalog and no file is
    /// touched, ever; references to it read absent from then on. Answers the status after the change, what now points
    /// at the name, and whether it was removed now or only saved — a name not registered while `library.json` is
    /// `unsaved` and its last saved list still names it repeats a removal whose save failed (ADR-0400 decision 14). The
    /// mount-change mutex is held across the change and the save that follows it.
    ///
    /// Refusals: the built-in `engram` `Conflict` · [`MOUNT_READ_ONLY`]; another name the rules refuse `InvalidArgument`
    /// · [`MOUNT_NAME_INVALID`]; a read-only `library.json` `Unavailable` · [`LIBRARY_CONFIG_READ_ONLY`]; a name not
    /// registered (outside that repeat) `NotFound` · [`MOUNT_NOT_FOUND`]; a save refused or failed after the change
    /// `Unavailable` · [`LIBRARY_CONFIG_WRITE_FAILED`]; the load-time judgement unfinished as for
    /// [`LibraryService::add_mount`].
    // ADR-0381
    // ADR-0400 decisions 11, 14 and 15
    pub fn remove_mount(&self, mount: &str) -> Result<RemoveAnswer, LibraryRefusal> {
        let name = self.named_mount(mount)?;
        let _serial = self.take_mount_change(&self.io.begin_read())?;
        self.refuse_read_only_config()?;
        let live = self
            .gate
            .with_read(|state| state.mounts.get(name.as_str()).cloned());
        let outcome = match live {
            Some(live) => {
                self.gate
                    .change_mount(MountChange::Remove(live.name.clone()), |_| {
                        unreachable!("a removal walks nothing")
                    })
                    .map_err(|_| mount_not_found(&name))?;
                self.release_overlap(&live.name);
                tracing::info!(
                    mount = live.name.as_str(),
                    path = %escape_controls(&live.folder.to_string_lossy()),
                    "library mount removed"
                );
                self.save_mounts_or_refuse(&format!(
                    "the library mount {} is removed but not saved",
                    live.name.as_str()
                ))?;
                Removal::Removed
            }
            None if self.config.unsaved_removal(&name) => {
                self.save_mounts_or_refuse(&format!(
                    "the library mount {} is removed but not saved",
                    name.as_str()
                ))?;
                tracing::info!(
                    mount = name.as_str(),
                    "library mount removal saved on a repeat"
                );
                Removal::SavedOnly
            }
            None => return Err(mount_not_found(&name)),
        };
        Ok(RemoveAnswer {
            now_absent: self.pointing_at(&name),
            outcome,
            status: self.status(),
        })
    }

    /// Re-points the mount `mount` at the folder `path` under the same name (ADR-0400 decision 13), creating it first
    /// when `create`; references do not change. The order and the refusals are [`LibraryService::add_mount`]'s, except
    /// that the mount must be registered (`NotFound` · [`MOUNT_NOT_FOUND`]), the built-in `engram` refuses `Conflict` ·
    /// [`MOUNT_READ_ONLY`], the overlap is judged against every other mount, and the mount
    /// already at that folder is a repeat (nothing changes but the save) unless it is held as overlapping another one
    /// at load — then it is judged and walked again. The relocated mount is walked and replaces its range.
    // ADR-0381
    // ADR-0400 decisions 10, 13, 14 and 15
    pub fn relocate_mount(
        &self,
        mount: &str,
        path: &Path,
        create: bool,
    ) -> Result<LibraryStatus, LibraryRefusal> {
        let name = self.named_mount(mount)?;
        let folder = absolute_folder(path)?;
        // ADR-0360: the load-time judgement and the folder's own checks share one read budget.
        let read = self.io.begin_read();
        let _serial = self.take_mount_change(&read)?;
        self.refuse_read_only_config()?;
        let Some(live) = self
            .gate
            .with_read(|state| state.mounts.get(name.as_str()).cloned())
        else {
            return Err(mount_not_found(&name));
        };
        let in_place = same_folder(&live.folder, &folder);
        if in_place && self.held_overlap(&live).is_none() {
            self.save_after(&live.name, &folder, "relocated")?;
            return Ok(self.status());
        }
        self.judge_new_folder(&read, &folder, Some(&live.name), create)?;
        // ADR-0400 decision 10: a held mount judged free at the folder it is registered at is no longer held — ended
        //   before the walk, so a reader's stat during the walk is not told the stale hold and cannot record it over
        //   the walk's state; should the walk fail, the mount stays registered there, judged free.
        if in_place {
            self.release_overlap(&live.name);
        }
        let change = MountChange::Relocate {
            name: live.name.clone(),
            folder: folder.clone(),
        };
        self.gate
            .change_mount(change, |mount| self.build_scan(mount, None, &Tags::new()))
            .map_err(|why| root_unusable(&folder, why))?;
        self.release_overlap(&live.name);
        tracing::info!(
            mount = live.name.as_str(),
            path = %escape_controls(&folder.to_string_lossy()),
            "library mount relocated"
        );
        self.save_after(&live.name, &folder, "relocated")?;
        Ok(self.status())
    }

    /// The mount name a remove, relocate or rename names: the built-in `engram` refuses `Conflict` · [`MOUNT_READ_ONLY`]
    /// (ADR-0400 decision 20), another name the rules refuse `InvalidArgument` · [`MOUNT_NAME_INVALID`].
    fn named_mount(&self, text: &str) -> Result<MountName, LibraryRefusal> {
        if is_builtin(text) {
            return Err(mount_read_only(text));
        }
        MountName::parse(text).map_err(|why| mount_name_invalid(text, &why))
    }

    /// `Unavailable` · [`LIBRARY_CONFIG_READ_ONLY`] while `library.json` is read-only for the run — every mount command
    /// refuses so before anything (ADR-0400 decision 15).
    fn refuse_read_only_config(&self) -> Result<(), LibraryRefusal> {
        match self.config.read_only() {
            Some(why) => Err(LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_CONFIG_READ_ONLY,
                json!({ "why": why }),
            )),
            None => Ok(()),
        }
    }

    /// The checks a folder passes before it is published as a mount, on the command's read budget `read` (the load-time
    /// judgement charged it first; the stat and every ask of the overlap judgement share the rest), and its `create`:
    /// [`mounts::overlap`] against every mount but `except` (the mount a relocate moves), then the folder created when
    /// `create`, then its stat. Without `create` the stat comes first (a missing folder answers its stat's words). With
    /// it the judgement comes first, on the folder as it will be (its nearest ancestor with a canonical path), and the
    /// folder is created only once it passed, so a refused judgement leaves nothing on disk. `Conflict` · [`MOUNT_OVERLAPS`]
    /// when it overlaps; `Unavailable` · [`LIBRARY_ROOT_UNUSABLE`] when the judgement cannot be made (fail closed —
    /// never taken as free), it cannot be created, or it does not stat as a folder. Holds no library lock.
    // ADR-0400 decision 10
    fn judge_new_folder(
        &self,
        read: &LaunchIo,
        folder: &Path,
        except: Option<&MountName>,
        create: bool,
    ) -> Result<(), LibraryRefusal> {
        if !create {
            self.stat_new_mount(read, folder)
                .map_err(|why| root_unusable(folder, why))?;
        }
        let others: Vec<Mount> = self.gate.with_read(|state| {
            state
                .mounts
                .iter()
                .filter(|mount| except.is_none_or(|except| mount.name.key() != except.key()))
                .cloned()
                .collect()
        });
        let probe = DiskProbe { io: &self.io, read };
        match overlap(folder, create, &others, &probe) {
            Ok(None) => {}
            Ok(Some(mount)) => return Err(mount_overlaps(folder, &mount)),
            Err(why) => return Err(root_unusable(folder, why)),
        }
        if create {
            std::fs::create_dir_all(folder).map_err(|e| root_unusable(folder, e.to_string()))?;
            self.stat_new_mount(read, folder)
                .map_err(|why| root_unusable(folder, why))?;
        }
        Ok(())
    }

    /// Saves every mount after a mount change of `name` to `folder` (`did` says which), or refuses `Unavailable` ·
    /// [`LIBRARY_CONFIG_WRITE_FAILED`] — the change stays live and a repeat saves again.
    fn save_after(&self, name: &MountName, folder: &Path, did: &str) -> Result<(), LibraryRefusal> {
        self.save_mounts_or_refuse(&format!(
            "the library mount {} {did} at {} is live but not saved",
            name.as_str(),
            folder.display()
        ))
    }

    /// [`LibraryService::save_mounts`], its failure as `Unavailable` · [`LIBRARY_CONFIG_WRITE_FAILED`] whose `why`
    /// starts with `live` and says the next start follows the file left.
    // ADR-0381
    fn save_mounts_or_refuse(&self, live: &str) -> Result<(), LibraryRefusal> {
        self.save_mounts().map_err(|why| {
            LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_CONFIG_WRITE_FAILED,
                json!({
                    "why": format!(
                        "{live}; the next start follows library.json as it is; repeat the call to save: {why}"
                    )
                }),
            )
        })
    }

    /// What points at the mount `name` from outside it: the references in the preset files of the other mounts (each
    /// listed entry counts) and the agents with an attachment in it. A removed mount's references now read absent; an
    /// added one's now resolve into its folder (ADR-0400 decisions 9 and 11). The registry is read under the library
    /// read lock.
    fn pointing_at(&self, name: &MountName) -> Pointing {
        let inside = |path: &LibPath| {
            let first = path.as_str().split('/').next().unwrap_or_default();
            path_key(Path::new(first)) == name.key()
        };
        self.gate.with_read(|state| {
            let presets = state
                .catalog
                .items()
                .filter(|item| !inside(&item.path))
                .filter_map(|item| match &item.content {
                    ItemContent::Preset { doc, .. } => Some(preset_refs(doc).filter(|r| inside(r)).count()),
                    _ => None,
                })
                .sum();
            let agents = self
                .values
                .registry
                .list()
                .iter()
                .filter(|profile| {
                    profile.presets.iter().any(|attached| {
                        matches!(AssetRef::parse(&attached.path), Ok(AssetRef::Library(path)) if inside(&path))
                    })
                })
                .count();
            Pointing { presets, agents }
        })
    }

    /// The first-start step: when `library.json` was missing at load, creates the default place, stats it as a new
    /// mount is stat'ed, publishes it as the mount `default_library` without a walk and only then writes it with
    /// `"default"`; the boot scan walks it ([`LibraryService::start_boot_scan`]). A failed create or stat writes
    /// nothing and leaves no mount, and the next start tries again; a failed write leaves the default mount live this
    /// run (`unsaved`, logged), and the next start follows the file that is left. A walk that fails later leaves the
    /// file written. A configured mount folder that vanished is never re-created (only a missing `library.json` makes
    /// the step due). The daemon calls it once, before the commands are served.
    // ADR-0312
    // ADR-0366
    // ADR-0375
    // ADR-0381
    // ADR-0382
    // ADR-0400
    pub fn first_start(&self) -> FirstStart {
        // ADR-0382 decision 1: not `take_mount_change` — the load-time judgement must not run before the daemon
        //   answers; the first start publishes into an empty list, where nothing overlaps.
        let _serial = self.lock_mount_change();
        if !self.config.first_start_due() {
            return FirstStart::NotDue;
        }
        let root = self.values.default_root.clone();
        let failed = |why: String| {
            tracing::warn!(root = %root.display(), "library first start failed; nothing published: {why}");
            FirstStart::Failed(why)
        };
        if let Err(e) = std::fs::create_dir_all(&root) {
            return failed(format!("create failed: {e}"));
        }
        if let Err(why) = self.stat_new_mount(&self.io.begin_read(), &root) {
            return failed(why);
        }
        // ADR-0382: not a walked mount change — the default place may already hold files (a first start run again
        //   after a failed save), and the first scan must walk them.
        let mount = Mount {
            name: MountName::first(),
            folder: root.clone(),
        };
        if let Err(why) = self.gate.publish_mount_unwalked(mount) {
            return failed(why);
        }
        match self.save_mounts() {
            Ok(()) => {
                tracing::info!(
                    root = %root.display(),
                    "library first start: mount `default_library` published at the default place"
                );
                FirstStart::Published
            }
            Err(why) => {
                tracing::warn!(
                    root = %root.display(),
                    "library first start: default mount live but not saved: {why}"
                );
                FirstStart::Unsaved(why)
            }
        }
    }

    /// The default place spelled as a mount's folder is compared with it.
    fn default_place(&self) -> PathBuf {
        normalize_folder(&self.values.default_root)
            .unwrap_or_else(|_| self.values.default_root.clone())
    }

    /// Whether `folder` is the default place — the same normalized spelling, compared by path key (a junction or
    /// short-name spelling of it is not): such a mount is saved as `"default"`.
    // ADR-0375
    fn is_default_place(&self, folder: &Path) -> bool {
        let folder = normalize_folder(folder).unwrap_or_else(|_| folder.to_path_buf());
        same_path(&folder, &self.default_place())
    }

    /// Saves the published mounts to `library.json`: a mount at the default place ([`LibraryService::is_default_place`])
    /// as `"default"`, every other one as its folder. Call holding the mount-change mutex, after the publish.
    // ADR-0375
    // ADR-0400
    fn save_mounts(&self) -> Result<(), String> {
        let settings: Vec<MountSetting> = self.gate.with_read(|state| {
            state
                .mounts
                .iter()
                .map(|mount| MountSetting {
                    name: mount.name.clone(),
                    place: match self.is_default_place(&mount.folder) {
                        true => MountPlace::Default,
                        false => MountPlace::Explicit(
                            normalize_folder(&mount.folder)
                                .unwrap_or_else(|_| mount.folder.clone()),
                        ),
                    },
                })
                .collect()
        });
        self.config.save(&settings)
    }

    /// Starts the boot scan of the published mounts as a background job on the service's spawner and returns at once.
    /// The job first runs the load-time overlap judgement ([`LibraryService::judge_at_load`]) unless a reader, a
    /// launch, `status` or a mount command already ran it, then passes the first-scan entrance
    /// ([`LibraryService::first_scan_join`]), so it joins a first scan a reader already started instead of walking
    /// again, and readers arriving during its walk wait for it there. With no published mount no job is started; a job
    /// that cannot be started is logged, and the first stat of a mount runs the judgement and the first reader the
    /// scan. A failed scan is logged; the next reader walks again. The daemon calls it on every start, after the
    /// first-start step, and never waits for it.
    // ADR-0382
    // ADR-0400 decision 10
    pub fn start_boot_scan(self: &Arc<Self>) {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        if mounts.is_empty() {
            tracing::debug!("library boot scan: no mount; not started");
            return;
        }
        let names: Vec<&str> = mounts.iter().map(|mount| mount.name.as_str()).collect();
        let names = escape_controls(&names.join(", "));
        let service = Arc::clone(self);
        let job = Box::new(move || {
            tracing::info!(mounts = %names, "library boot scan started");
            let started = std::time::Instant::now();
            service.judge_at_load(&service.io.begin_read());
            match service.first_scan_join() {
                Ok(()) => {
                    tracing::info!(
                        mounts = %names,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "library boot scan done"
                    );
                    // ADR-0400 decision 17: a mount the scan could not walk is that mount's state; its next reader
                    //   walks it again.
                    for left in service.gate.with_read(unavailable_mounts) {
                        tracing::warn!(
                            mount = %escape_controls(&left.mount),
                            "library mount not walked at boot: {}",
                            escape_controls(left.state.why())
                        );
                    }
                }
                Err(why) => tracing::warn!(
                    mounts = %names,
                    "library boot scan failed; the next reader scans again: {}",
                    escape_controls(&why)
                ),
            }
        });
        if let Err(e) = self.io.spawner().spawn(BOOT_SCAN_JOB, job) {
            tracing::warn!("library-boot-scan spawn failed; the first reader scans: {e}");
        }
    }

    /// The load-time overlap check, once per run (ADR-0400 decision 10): the registered mounts judged against each
    /// other ([`mounts::overlapping`], holding no library lock); each one overlapping an earlier one is held — kept
    /// registered, never walked, `unreadable` with a `why` naming the other ([`LibraryService::held_overlap`]) — until
    /// it is relocated or removed. Every stat of a mount ([`LibraryService::stat_held`]) and every mount command
    /// ([`LibraryService::take_mount_change`]) calls it first, so whoever comes first — the boot scan's job, a reader's
    /// scan, a read entrance, a launch, `status`, a mount command — runs it, and the others wait for that one: no mount
    /// is walked or read before it was judged, and no mount command changes the list between the judgement and its
    /// holds. Only the daemon's requests and the boot job reach it, never the boot path itself (ADR-0382 decision 1).
    ///
    /// It costs no caller more than its own budget (ADR-0360): the one that runs it spends at most what is left of
    /// `io`'s required budget (and at most one limit) on it, one that finds it running waits at most that much, and
    /// either is charged the time ([`LaunchIo::charge_required`]), so its own asks get only the rest. `false` = it is
    /// still running when `io`'s budget is spent — the caller must not take any mount as `Ok`. Call it holding no
    /// library lock and no ticket; under the mount-change mutex only once it is done (it then returns at once).
    // ADR-0360
    // ADR-0382 decision 1
    // ADR-0400 decision 10
    fn judge_at_load(&self, io: &LaunchIo) -> bool {
        let began = std::time::Instant::now();
        let mut state = lock(&self.load_judgement);
        if *state == Judgement::Done {
            return true;
        }
        #[cfg(test)]
        let mut counted = false;
        let judged = loop {
            let left = io.required_left().saturating_sub(began.elapsed());
            match *state {
                Judgement::Done => break true,
                Judgement::NotStarted => {
                    *state = Judgement::Running;
                    drop(state);
                    let run = JudgementRun {
                        service: self,
                        done: false,
                    };
                    self.hold_overlapping(&self.io.begin_read_within(left));
                    run.finish();
                    state = lock(&self.load_judgement);
                }
                Judgement::Running if left.is_zero() => break false,
                Judgement::Running => {
                    #[cfg(test)]
                    if !std::mem::replace(&mut counted, true) {
                        self.load_judge_blocked
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    state = wait_timeout(&self.load_judged, state, left).0;
                }
            }
        };
        drop(state);
        io.charge_required(began.elapsed());
        judged
    }

    /// The body of [`LibraryService::judge_at_load`], its asks on `read`. No mount is walked before it: nothing to
    /// record in the gate.
    fn hold_overlapping(&self, read: &LaunchIo) {
        let mounts = self.gate.with_read(|state| state.mounts.clone());
        if mounts.iter().nth(1).is_none() {
            return;
        }
        let found = overlapping(&mounts, &DiskProbe { io: &self.io, read });
        let mut overlapping = lock(&self.overlapping);
        for (name, other) in found {
            let Some(mount) = mounts.get(name.as_str()) else {
                continue;
            };
            let why = format!(
                "overlaps the library mount {}; relocate or remove it",
                other.as_str()
            );
            tracing::warn!(
                mount = name.as_str(),
                with = other.as_str(),
                "library mount overlaps an earlier one; not walked"
            );
            overlapping.push((mount.clone(), why));
        }
    }

    /// Why `mount` is held as overlapping another mount at load, if it is (the same name at the same folder).
    fn held_overlap(&self, mount: &Mount) -> Option<String> {
        lock(&self.overlapping)
            .iter()
            .find(|(held, _)| {
                held.name.key() == mount.name.key() && same_path(&held.folder, &mount.folder)
            })
            .map(|(_, why)| why.clone())
    }

    /// Ends the hold of the mount `name` — it was removed or relocated. (An add never needs it: a hold is set only on
    /// a registered mount, before any mount command, and a removal ends it.)
    fn release_overlap(&self, name: &MountName) {
        lock(&self.overlapping).retain(|(held, _)| held.name.key() != name.key());
    }

    /// The mount-change mutex of a mount command, taken after the load-time judgement
    /// ([`LibraryService::judge_at_load`] on `io`, the command's read budget). `Unavailable` · [`LIBRARY_UNAVAILABLE`]
    /// when the judgement is still running once that budget is spent — nothing done; a repeat waits again.
    fn take_mount_change(&self, io: &LaunchIo) -> Result<MutexGuard<'_, ()>, LibraryRefusal> {
        if !self.judge_at_load(io) {
            return Err(LibraryRefusal::unavailable(
                "the load-time overlap check of the library mounts has not finished; repeat the call".to_owned(),
            ));
        }
        Ok(self.lock_mount_change())
    }

    fn lock_mount_change(&self) -> MutexGuard<'_, ()> {
        #[cfg(test)]
        if matches!(
            self.mount_change.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            self.mount_change_blocked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        lock(&self.mount_change)
    }

    /// The whole folder tree, by path: each registered mount's top with its last known state (no fresh stat) and the
    /// built-in mount `engram` (always `Ok`), every folder the catalog holds plus every skill folder of the mounts that
    /// can be read, and the built-in's virtual folders; a skill folder is marked.
    /// The mounts that cannot be read are named in `unavailable` and their folders left out — the answer is partial,
    /// never a refusal for them (ADR-0400 decision 6). After the first-scan entrance (its failure `Unavailable` ·
    /// [`LIBRARY_UNAVAILABLE`]), under one read lock; with no mount registered the tree is empty.
    pub fn folders(&self) -> Result<FoldersAnswer, LibraryRefusal> {
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        Ok(self.gate.with_read(|state| {
            let catalog = &state.catalog;
            let readable = |path: &LibPath| {
                state
                    .mounts
                    .locate(path)
                    .is_some_and(|(mount, _)| state.seen(&mount.name).usable())
            };
            let mut seen = std::collections::HashSet::new();
            let mut folders: Vec<FolderNode> = mount_rows(state)
                .map(|(top, mount)| {
                    seen.insert(key(&top));
                    FolderNode {
                        path: top.as_str().to_owned(),
                        name: top.file_name().to_owned(),
                        skill: false,
                        mount_state: Some(mount.shown()),
                    }
                })
                .collect();
            let skills = catalog
                .items()
                .filter(|item| item.kind == ItemKind::Skill)
                .map(|item| &item.path);
            folders.extend(
                catalog
                    .folders()
                    .chain(skills)
                    .filter(|folder| readable(folder))
                    .filter(|folder| seen.insert(key(folder)))
                    .map(|folder| FolderNode {
                        path: folder.as_str().to_owned(),
                        name: folder.file_name().to_owned(),
                        skill: catalog
                            .get(&key(folder))
                            .is_some_and(|item| item.kind == ItemKind::Skill),
                        mount_state: None,
                    }),
            );
            // ADR-0400 decision 20: the built-in mount is always present, with its virtual folders.
            folders.push(FolderNode {
                path: BUILTIN_MOUNT.to_owned(),
                name: BUILTIN_MOUNT.to_owned(),
                skill: false,
                mount_state: Some(self.builtin_mount_state()),
            });
            folders.extend(builtin_folders().into_iter().map(|folder| FolderNode {
                path: folder.as_str().to_owned(),
                name: folder.file_name().to_owned(),
                skill: false,
                mount_state: None,
            }));
            folders.sort_by(|a, b| a.path.cmp(&b.path));
            FoldersAnswer {
                folders,
                unavailable: self.whole_tree_unavailable(state),
            }
        }))
    }

    /// One item's detail from the catalog (no disk read): its `list` row, its absolute path in its mount's folder, who
    /// refers to it directly and indirectly, an instruction's excerpt and a settings asset's raw-field names. After the
    /// first-scan entrance (its failure `Unavailable` · [`LIBRARY_UNAVAILABLE`]); a path the catalog does not hold, or
    /// whose mount is not registered, `NotFound` · `not-found`. A built-in answers from the built-ins with no entrance
    /// ([`LibraryService::read_builtin`]).
    pub fn read(&self, path: &LibPath) -> Result<ReadAnswer, LibraryRefusal> {
        let not_found = || {
            LibraryRefusal::new(
                RefusalClass::NotFound,
                NOT_FOUND,
                json!({ "path": path.as_str() }),
            )
        };
        // ADR-0400 decision 4: a mount's top is not an item, whatever its mount's state.
        if path.parent().is_none() {
            return Err(not_found());
        }
        if in_builtin(path) {
            return match AssetRef::parse(path.as_str()) {
                Ok(AssetRef::BuiltIn(id)) => self.read_builtin(id),
                _ => Err(not_found()),
            };
        }
        self.first_scan_join()
            .map_err(LibraryRefusal::unavailable)?;
        // ADR-0400 decision 6: an item of a mount that cannot be read refuses, naming it.
        self.refuse_unusable_mount(path)?;
        self.gate.with_read(|state| {
            let catalog = &state.catalog;
            let item = catalog.get(&key(path)).ok_or_else(not_found)?;
            let absolute = state.mounts.disk_path(&item.path).ok_or_else(not_found)?;
            let registry = &self.values.registry;
            let (reaching, agents_direct) = referrers_of(catalog, registry, &item.path);
            let lists_it = |preset: &LibPath| match catalog.get(&key(preset)).map(|p| &p.content) {
                Some(ItemContent::Preset { doc, .. }) => {
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
                absolute,
                presets_direct: names(direct),
                presets_indirect: names(indirect),
                agents_direct,
                agents_indirect,
                excerpt,
                settings_fields,
            })
        })
    }

    /// Walks and reads `under` (`None` = its whole folder) in `mount` outside every lock.
    fn build_scan(
        &self,
        mount: &Mount,
        under: Option<&MountPath>,
        tags: &Tags,
    ) -> io::Result<ScanResult> {
        let read = |path: &Path| self.store.read(path, LIBRARY_READ_CAP);
        self.build_scan_reading(mount, under, tags, &read)
    }

    /// [`LibraryService::build_scan`] with the file reads going through `read` (the launch overlay keeps the bytes).
    fn build_scan_reading(
        &self,
        mount: &Mount,
        under: Option<&MountPath>,
        tags: &Tags,
        read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
    ) -> io::Result<ScanResult> {
        // The racy rule's reference — taken before the walk (`ScanResult::build`).
        let now = SystemTime::now();
        let listing = self
            .store
            .scan(&mount.folder, under, &|| self.gate.next_seq())?;
        Ok(ScanResult::build(
            listing,
            &mount.name,
            under,
            tags,
            read,
            &*self.values.backends,
            now,
        ))
    }

    /// One path read through the same partial scan a refresh of it would run — so it admits exactly what the scan
    /// admits (names, links, kinds), and every read draws its number first. A path in no mount of `mounts` is not read.
    fn observe(&self, mounts: &Mounts, path: &LibPath) -> Observed {
        let Some((mount, under)) = mounts.locate(path) else {
            return Observed::Refused;
        };
        match self.build_scan(mount, under.as_ref(), &Tags::new()) {
            Err(e) => {
                tracing::debug!(
                    path = path.as_str(),
                    "build_scan failed; not read (Observed::Refused): {}",
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

    /// The list answer from the catalog. A query without a folder answers from the mounts that can be read and names
    /// the others in `unavailable`; the virtual root's folder rows are the registered mounts with their last known
    /// state (ADR-0400 decisions 4 and 6).
    fn answer(&self, state: &State, folder: Option<&LibPath>, filter: &ListFilter) -> ListAnswer {
        let query = filter.query.as_deref().map(str::to_lowercase);
        let readable = |path: &LibPath| {
            state
                .mounts
                .locate(path)
                .is_some_and(|(mount, _)| state.seen(&mount.name).usable())
        };
        let items = match &query {
            None => state.catalog.children(folder),
            Some(_) => {
                // Strictly below: the folder asked for is not a result of its own search.
                let mut below: Vec<&Item> = state
                    .catalog
                    .items()
                    .filter(|item| within(&item.path, folder))
                    .filter(|item| folder.is_none_or(|folder| key(&item.path) != key(folder)))
                    .filter(|item| readable(&item.path))
                    .collect();
                below.sort_by(|a, b| a.path.cmp(&b.path));
                below
            }
        };
        // The references are counted only for the rows kept — each count reads the registry.
        let mut rows: Vec<ListRow> = items
            .into_iter()
            .map(|item| (item, row(item, 0)))
            .filter(|(_, row)| filter.keeps(row, query.as_deref()))
            .map(|(item, mut row)| {
                row.references = reference_count(&state.catalog, &self.values.registry, &item.path);
                row
            })
            .collect();
        // ADR-0400 decision 20: a search of the whole tree searches the built-in mount too.
        if let (Some(_), None) = (&query, folder) {
            rows.extend(self.builtin_rows(None, filter));
            rows.sort_by(|a, b| a.path.cmp(&b.path));
        }
        let folders = match (&query, folder) {
            (Some(_), _) => Vec::new(),
            (None, None) => {
                let mut tops: Vec<FolderRow> = mount_rows(state)
                    .map(|(top, seen)| FolderRow {
                        path: top.as_str().to_owned(),
                        name: top.file_name().to_owned(),
                        mount_state: Some(seen.shown()),
                    })
                    .collect();
                // ADR-0400 decision 20: the built-in mount is always present.
                tops.push(FolderRow {
                    path: BUILTIN_MOUNT.to_owned(),
                    name: BUILTIN_MOUNT.to_owned(),
                    mount_state: Some(self.builtin_mount_state()),
                });
                tops.sort_by(|a, b| a.path.cmp(&b.path));
                tops
            }
            (None, Some(folder)) => {
                let want = key(folder);
                let mut subfolders: Vec<&LibPath> = state
                    .catalog
                    .folders()
                    .filter(|sub| sub.parent().as_ref().map(key) == Some(want.clone()))
                    .filter(|sub| state.catalog.get(&key(sub)).is_none())
                    .collect();
                subfolders.sort();
                subfolders
                    .into_iter()
                    .map(|sub| FolderRow {
                        path: sub.as_str().to_owned(),
                        name: sub.file_name().to_owned(),
                        mount_state: None,
                    })
                    .collect()
            }
        };
        let unavailable = match (&query, folder) {
            (Some(_), None) => self.whole_tree_unavailable(state),
            _ => Vec::new(),
        };
        ListAnswer {
            folder: folder.cloned(),
            rows,
            folders,
            unavailable,
        }
    }

    /// The built-in mount's state, judged from the built-ins read once at start ([`Builtins::folder_unreadable`]):
    /// unreadable when the run has no built-in folder or not one built-in file in it could be read — `read` of a
    /// built-in then refuses — else `Ok`. Its `list` / `folders` row and a whole-tree read's `unavailable` carry it;
    /// its item rows stay, as stand-ins with their `builtin-unavailable` state.
    // ADR-0400 decision 20
    fn builtin_mount_state(&self) -> MountState {
        match self.builtins.folder_unreadable() {
            None => MountState::Ok,
            Some(why) => MountState::Unreadable {
                why: why.to_owned(),
            },
        }
    }

    /// A whole-tree read's `unavailable`: the library mounts that cannot be read ([`unavailable_mounts`]) and the
    /// built-in mount when it cannot be ([`LibraryService::builtin_mount_state`]), by name.
    fn whole_tree_unavailable(&self, state: &State) -> Vec<UnavailableMount> {
        let mut unavailable = unavailable_mounts(state);
        if let state @ MountState::Unreadable { .. } = self.builtin_mount_state() {
            unavailable.push(UnavailableMount {
                mount: BUILTIN_MOUNT.to_owned(),
                state,
            });
            unavailable.sort_by(|a, b| a.mount.cmp(&b.mount));
        }
        unavailable
    }

    /// The list answer of `folder` in the built-in mount (ADR-0400 decision 20): the built-ins directly in it — with a
    /// query, every matching one below it — kept by `filter`, and its virtual folders directly in it (none with a
    /// query). Its mount segment is matched as [`builtin_rest`] matches it, what follows exactly — the rule `read`
    /// uses. A folder that holds no built-in answers nothing.
    fn builtin_answer(&self, folder: &LibPath, filter: &ListFilter) -> ListAnswer {
        let inside = builtin_rest(folder).unwrap_or_default();
        let folders = match filter.query {
            Some(_) => Vec::new(),
            None => builtin_folders()
                .into_iter()
                .filter(|sub| builtin_rest(sub).map(parent_of) == Some(inside))
                .map(|sub| FolderRow {
                    path: sub.as_str().to_owned(),
                    name: sub.file_name().to_owned(),
                    mount_state: None,
                })
                .collect(),
        };
        ListAnswer {
            folder: Some(folder.clone()),
            rows: self.builtin_rows(Some(inside), filter),
            folders,
            unavailable: Vec::new(),
        }
    }

    /// The rows of the built-ins directly in the built-in mount's folder `inside` (its path inside the mount, `""` =
    /// its top; `None` = the virtual root: none) — with a query, of every built-in strictly below it that matches, as a
    /// library folder answers — kept by `filter`, by path. `inside` is matched exactly, as `read` matches an id.
    fn builtin_rows(&self, inside: Option<&str>, filter: &ListFilter) -> Vec<ListRow> {
        let query = filter.query.as_deref().map(str::to_lowercase);
        BuiltinId::ALL
            .into_iter()
            .filter(|id| {
                let id = id.id();
                match (&query, inside) {
                    (None, None) => false,
                    (None, Some(inside)) => parent_of(id) == inside,
                    (Some(_), None) => true,
                    (Some(_), Some(inside)) => {
                        inside.is_empty()
                            || id
                                .strip_prefix(inside)
                                .is_some_and(|tail| tail.starts_with('/'))
                    }
                }
            })
            .map(|id| self.builtin_row(id))
            .filter(|row| filter.keeps(row, query.as_deref()))
            .collect()
    }

    /// The row of the built-in `id`: named by the last name of its path, its `description` (the priming: its
    /// excerpt), a stand-in's `builtin-unavailable` reason as its state, and as references the agents whose Default
    /// slot is on for the built-in preset ([`default_slot_count`]); a library preset that lists a built-in is not
    /// counted — the catalog indexes library references only.
    // ADR-0386
    // ADR-0400 decision 20
    fn builtin_row(&self, id: BuiltinId) -> ListRow {
        let view = self.builtins.view(id);
        let path = id.path();
        let description = match view.content {
            ItemContent::Preset { doc, .. } => doc.description.clone().unwrap_or_default(),
            ItemContent::Settings { doc, .. } => doc.description.clone().unwrap_or_default(),
            ItemContent::Instruction { excerpt } => excerpt.clone(),
            _ => String::new(),
        };
        let state = view.stand_in.map(|why| Reason {
            code: BUILTIN_UNAVAILABLE.to_owned(),
            args: json!({ "path": path.as_str(), "why": why }),
            line: None,
        });
        let references = match id {
            BuiltinId::DefaultPreset => default_slot_count(&self.values.registry),
            BuiltinId::Priming | BuiltinId::DefaultSettings => 0,
        };
        ListRow {
            path: path.as_str().to_owned(),
            kind: view.kind.clone(),
            name: path.file_name().to_owned(),
            state,
            references,
            description,
        }
    }

    /// `library.read` of the built-in `id`: its row, its file in the built-in folder (`<builtin>\presets\default.yaml`
    /// — where it was read from, or would have been for a stand-in), the agents whose Default slot is on for the
    /// built-in preset, the priming's excerpt and the settings' raw-field names. A built-in mount that cannot be read
    /// ([`LibraryService::builtin_mount_state`]) refuses `Unavailable` · [`LIBRARY_UNAVAILABLE`] `{mount, message}`.
    // ADR-0400 decision 20
    fn read_builtin(&self, id: BuiltinId) -> Result<ReadAnswer, LibraryRefusal> {
        let state = self.builtin_mount_state();
        let (MountState::Ok, Some(dir)) = (&state, &self.values.builtin_dir) else {
            let why = state.why();
            return Err(LibraryRefusal::new(
                RefusalClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                json!({
                    "mount": BUILTIN_MOUNT,
                    "message": format!("the library mount {BUILTIN_MOUNT} cannot be read: {why}"),
                }),
            ));
        };
        let view = self.builtins.view(id);
        let mut agents_direct: Vec<AgentId> = match id {
            BuiltinId::DefaultPreset => self
                .values
                .registry
                .attachments()
                .into_iter()
                .filter(|(_, attached)| attached.default_preset)
                .map(|(agent, _)| agent)
                .collect(),
            BuiltinId::Priming | BuiltinId::DefaultSettings => Vec::new(),
        };
        agents_direct.sort_unstable();
        let (excerpt, settings_fields) = match view.content {
            ItemContent::Instruction { excerpt } => (Some(excerpt.clone()), Vec::new()),
            ItemContent::Settings { doc, .. } => (
                None,
                doc.raw.iter().map(|field| field.name.clone()).collect(),
            ),
            _ => (None, Vec::new()),
        };
        Ok(ReadAnswer {
            row: self.builtin_row(id),
            absolute: file_of(id)
                .split('/')
                .fold(dir.clone(), |at, name| at.join(name)),
            presets_direct: Vec::new(),
            presets_indirect: Vec::new(),
            agents_direct,
            agents_indirect: Vec::new(),
            excerpt,
            settings_fields,
        })
    }
}

/// The parent of a path inside the built-in mount (`""` = the mount's top).
fn parent_of(inside: &str) -> &str {
    inside.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// The built-in mount's virtual folders below its top (`engram/settings`), each once, by path — the folders the
/// built-ins' paths lie in.
// ADR-0400 decision 20
fn builtin_folders() -> Vec<LibPath> {
    let mut folders: Vec<LibPath> = BuiltinId::ALL
        .into_iter()
        .filter_map(|id| id.path().parent())
        .filter(|folder| folder.parent().is_some())
        .collect();
    folders.sort();
    folders.dedup();
    folders
}

/// The refusal of a write into the built-in mount — `Conflict` · [`MOUNT_READ_ONLY`] (`args` = `{mount, path}`).
// ADR-0400 decision 20
pub(crate) fn mount_read_only(path: &str) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        MOUNT_READ_ONLY,
        json!({ "mount": BUILTIN_MOUNT, "path": escape_controls(path) }),
    )
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

/// Each registered mount's top with what the scans know of it, by name.
fn mount_rows(state: &State) -> impl Iterator<Item = (LibPath, MountSeen)> + '_ {
    let mut tops: Vec<(LibPath, MountSeen)> = state
        .mounts
        .iter()
        .map(|mount| (virtual_path(&mount.name, None), state.seen(&mount.name)))
        .collect();
    tops.sort_by(|a, b| a.0.cmp(&b.0));
    tops.into_iter()
}

/// The registered mounts whose items cannot be read, by name, with the state a reader is told.
// ADR-0400 decision 6
fn unavailable_mounts(state: &State) -> Vec<UnavailableMount> {
    let mut unavailable: Vec<UnavailableMount> = state
        .mounts
        .iter()
        .map(|mount| (mount, state.seen(&mount.name)))
        .filter(|(_, seen)| !seen.usable())
        .map(|(mount, seen)| UnavailableMount {
            mount: mount.name.as_str().to_owned(),
            state: seen.shown(),
        })
        .collect();
    unavailable.sort_by(|a, b| a.mount.cmp(&b.mount));
    unavailable
}

fn is_known(catalog: &Catalog, path: &LibPath) -> bool {
    let want = key(path);
    catalog.get(&want).is_some() || catalog.folders().any(|folder| key(folder) == want)
}

/// The registered mounts a computation over the presets `attached` reaches in `catalog`: the mount of each attached
/// library preset and of every library reference it reaches there by its want's tag rule ([`closure_by`] ·
/// [`TagRule::refs`]), each once, in first-reach order — the mounts a read computation stats (ADR-0400 decision 19).
/// A reference only another backend's tag lists is not followed: the resolution never looks it up.
pub(crate) fn reached_mounts(
    catalog: &Catalog,
    mounts: &Mounts,
    attached: &[(TagRule, &AssetRef)],
) -> Vec<Mount> {
    let mut seen = std::collections::HashSet::new();
    let mut reached = Vec::new();
    for (rule, preset) in attached {
        let AssetRef::Library(path) = preset else {
            continue;
        };
        let refs = closure_by(catalog, path, |doc| rule.refs(doc));
        let paths = std::iter::once(path).chain(refs.iter().filter_map(|r| match r {
            AssetRef::Library(path) => Some(path),
            AssetRef::BuiltIn(_) => None,
        }));
        for path in paths {
            if let Some((mount, _)) = mounts.locate(path) {
                if seen.insert(mount.name.key()) {
                    reached.push(mount.clone());
                }
            }
        }
    }
    reached
}

/// What a read computation keeps a mount's stat by ([`mount_key`]).
pub(crate) type ReadStats = std::collections::HashMap<(PathKey, PathKey), FolderStatus>;

/// The key of `mount`'s stat in [`ReadStats`]: its name key and its folder's path key — two mounts on one folder keep
/// their own answers (one may be held as overlapping the other), and a mount moved after its stat is not judged by it.
// ADR-0400 decision 10
pub(crate) fn mount_key(mount: &Mount) -> (PathKey, PathKey) {
    (mount.name.key(), path_key(&mount.folder))
}

/// The mount states of a read computation over `mounts`: a mount `stats` holds ([`mount_key`]) judged by
/// [`RootState::of`] with the first-scan answer `joined`; every other registered mount by its last known state
/// (`seen`) — usable when walked and `Ok`, else absent when last seen missing and unreadable otherwise, the rule
/// `list` and `read` use (ADR-0400 decision 17).
// ADR-0387
// ADR-0400 decision 19
pub(crate) fn read_states(
    mounts: &Mounts,
    seen: impl Fn(&MountName) -> MountSeen,
    stats: &ReadStats,
    joined: &Result<(), String>,
) -> MountStates {
    let mut states = MountStates::new(mounts, false);
    for mount in mounts.iter() {
        let state = match stats.get(&mount_key(mount)) {
            Some(stat) => RootState::of(joined, Some(stat)),
            None => {
                let seen = seen(&mount.name);
                match (seen.usable(), seen.shown()) {
                    (false, MountState::Missing { why }) => RootState::Absent(why),
                    (false, MountState::Unreadable { why }) => RootState::Unreadable(why),
                    _ => RootState::of(joined, None),
                }
            }
        };
        states.set(&mount.name, state);
    }
    states
}

/// [`LibraryService::refuse_unusable_mount`] inside a change, under its write lock and before its disk IO: `path`'s
/// mount judged by its recorded state only — no IO, no walk again. The entrance passed it before the ticket, but a scan
/// queued ahead of the change may have found the mount gone since. A path in no registered mount passes.
// ADR-0400 decision 6
pub(crate) fn refuse_unusable_now(
    mounts: &MountsNow,
    path: &LibPath,
) -> Result<(), LibraryRefusal> {
    let Some((mount, _)) = mounts.locate(path) else {
        return Ok(());
    };
    let seen = mounts.seen(&mount.name);
    match seen.usable() {
        true => Ok(()),
        false => Err(LibraryRefusal::Refused {
            class: RefusalClass::Unavailable,
            reason: mount_unavailable(&mount.name, &seen.shown()),
        }),
    }
}

/// The reason of a read whose range lies in mount `name` that cannot be read — [`LIBRARY_UNAVAILABLE`] with `{mount,
/// message}`.
// ADR-0400 decision 6
pub(crate) fn mount_unavailable(name: &MountName, state: &MountState) -> Reason {
    Reason {
        code: LIBRARY_UNAVAILABLE.to_owned(),
        args: json!({
            "mount": name.as_str(),
            "message": format!("the library mount {} cannot be read: {}", name.as_str(), state.why()),
        }),
        line: None,
    }
}

/// The refusal of a write whose target is the virtual root (`None`) or a mount's top — `InvalidArgument` ·
/// [`VIRTUAL_ROOT_READ_ONLY`].
// ADR-0400 decision 5
pub(crate) fn virtual_root_read_only(path: Option<&LibPath>) -> LibraryRefusal {
    let args = match path {
        Some(path) => json!({ "path": path.as_str() }),
        None => json!({}),
    };
    LibraryRefusal::new(RefusalClass::InvalidArgument, VIRTUAL_ROOT_READ_ONLY, args)
}

fn root_unusable(root: &Path, why: String) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Unavailable,
        LIBRARY_ROOT_UNUSABLE,
        json!({ "path": root.display().to_string(), "why": why }),
    )
}

/// `path` normalized as a mount's folder; a path that is not absolute refuses `InvalidArgument` ·
/// `item-path-invalid` `{path, why}`.
fn absolute_folder(path: &Path) -> Result<PathBuf, LibraryRefusal> {
    normalize_folder(path).map_err(|why| {
        LibraryRefusal::new(
            RefusalClass::InvalidArgument,
            ITEM_PATH_INVALID,
            json!({
                "path": escape_controls(&path.display().to_string()),
                "why": why.to_string(),
            }),
        )
    })
}

/// Whether two mount folders are one: their normalized spellings compared by path key.
fn same_folder(a: &Path, b: &Path) -> bool {
    let normalized = |path: &Path| normalize_folder(path).unwrap_or_else(|_| path.to_path_buf());
    same_path(&normalized(a), &normalized(b))
}

/// `InvalidArgument` · [`MOUNT_NAME_INVALID`] `{name, why}`.
fn mount_name_invalid(text: &str, why: &MountNameError) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::InvalidArgument,
        MOUNT_NAME_INVALID,
        json!({ "name": escape_controls(text), "why": why.to_string() }),
    )
}

/// `Conflict` · [`MOUNT_NAME_TAKEN`] `{name}`.
fn mount_name_taken(name: &MountName) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::Conflict,
        MOUNT_NAME_TAKEN,
        json!({ "name": name.as_str() }),
    )
}

/// `NotFound` · [`MOUNT_NOT_FOUND`] `{mount}`.
fn mount_not_found(name: &MountName) -> LibraryRefusal {
    LibraryRefusal::new(
        RefusalClass::NotFound,
        MOUNT_NOT_FOUND,
        json!({ "mount": name.as_str() }),
    )
}

/// `Conflict` · [`MOUNT_OVERLAPS`] `{path, with: "mount", mount}`.
fn mount_overlaps(folder: &Path, mount: &MountName) -> LibraryRefusal {
    let path = escape_controls(&folder.display().to_string());
    let args = json!({ "path": path, "with": "mount", "mount": mount.as_str() });
    LibraryRefusal::new(RefusalClass::Conflict, MOUNT_OVERLAPS, args)
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
        ItemContent::Preset { doc, .. } => (
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

/// What a link probe of a `not-found` library reference makes of it (ADR-0387): `Ok(Some)` — the path lies at or below
/// a link inside the library, which the scan never enters — is [`PATH_THROUGH_LINK`]; a path under a file
/// (`NotADirectory`) is absent, so `None` (the row stays `not-found`), as is `Ok(None)`; any other failure is
/// [`launch::LIBRARY_READ_FAILED`] — the path could not be checked, so it is not known to be absent. The one rule of
/// `LibraryService::mark_links` (the launch's and the answer's problems) and `mark_row_link` (a preset line's state),
/// so the two agree. `path` is the reference as written, `line` its preset line.
// ADR-0387
pub(crate) fn link_verdict(
    probe: io::Result<Option<MountPath>>,
    path: &str,
    line: Option<u32>,
) -> Option<Reason> {
    match probe {
        Ok(None) => None,
        Err(e) if e.kind() == io::ErrorKind::NotADirectory => None,
        Ok(Some(_)) => Some(Reason {
            code: PATH_THROUGH_LINK.to_owned(),
            args: json!({ "path": path }),
            line,
        }),
        Err(e) => Some(Reason {
            code: launch::LIBRARY_READ_FAILED.to_owned(),
            args: json!({ "path": path, "why": e.to_string() }),
            line,
        }),
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

    use engram_dashboard_agent::launch::LaunchKitSource;
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
        scans: Mutex<Vec<(PathBuf, Option<MountPath>)>>,
        read_caps: Mutex<Vec<u64>>,
        /// The next scan parks at `Start` (if armed) and then fails as an unreadable root.
        fail_next_scan: Mutex<bool>,
        /// Every scan fails as an unreadable root.
        fail_every_scan: Mutex<bool>,
        /// Every link probe fails with this error kind.
        fail_link: Mutex<Option<io::ErrorKind>>,
        /// Every path a store call other than a scan was asked about (a link probe logs its mount's folder).
        touched: Mutex<Vec<PathBuf>>,
        hold: Mutex<Hold>,
        moved: Condvar,
        /// The next read of this path parks: it tells the sender, then waits for the receiver ([`TestStore::door_read`]).
        read_door: Mutex<Option<ReadDoor>>,
    }

    type ReadDoor = (
        PathBuf,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );

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
                fail_every_scan: Mutex::default(),
                fail_link: Mutex::default(),
                touched: Mutex::default(),
                hold: Mutex::default(),
                moved: Condvar::new(),
                read_door: Mutex::default(),
            })
        }

        /// Arms a door on the next read of `path`: that read tells the returned receiver when it starts, then waits
        /// (at most 20 s) until the returned sender sends.
        fn door_read(
            &self,
            path: PathBuf,
        ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
            let (entered_tx, entered) = std::sync::mpsc::channel();
            let (release, release_rx) = std::sync::mpsc::channel();
            *lock(&self.read_door) = Some((path, entered_tx, release_rx));
            (entered, release)
        }

        fn pass_read_door(&self, path: &Path) {
            let door = {
                let mut door = lock(&self.read_door);
                match door.as_ref() {
                    Some((at, _, _)) if at == path => door.take(),
                    _ => None,
                }
            };
            if let Some((_, entered, release)) = door {
                entered.send(()).unwrap();
                release
                    .recv_timeout(Duration::from_secs(20))
                    .expect("the test opens the read door");
            }
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

        /// The store calls other than scans that reached a path at or below `root`.
        fn touched_under(&self, root: &str) -> Vec<PathBuf> {
            lock(&self.touched)
                .iter()
                .filter(|path| path.starts_with(root))
                .cloned()
                .collect()
        }

        fn note(&self, path: &Path) {
            lock(&self.touched).push(path.to_path_buf());
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
            under: Option<&MountPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            lock(&self.scans).push((root.to_path_buf(), under.cloned()));
            if std::mem::take(&mut *lock(&self.fail_next_scan)) || *lock(&self.fail_every_scan) {
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

        fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
            self.note(root);
            if let Some(kind) = *lock(&self.fail_link) {
                return Err(io::Error::from(kind));
            }
            self.at_root(root).first_link(root, rel)
        }

        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            lock(&self.read_caps).push(cap);
            self.note(path);
            self.pass_read_door(path);
            self.holding(path).read(path, cap)
        }

        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            self.note(path);
            self.holding(path).tag(path)
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.note(path);
            self.holding(path).write_atomic(path, bytes)
        }

        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.note(path);
            self.holding(path).create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.note(path);
            self.holding(path).create_dir(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.note(from);
            self.note(to);
            self.holding(from).rename(from, to)
        }

        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.note(from);
            self.note(to);
            self.holding(from).copy(from, to)
        }

        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            self.note(path);
            self.at_root(root).trash(root, path, bundle)
        }

        fn exists(&self, path: &Path) -> bool {
            self.note(path);
            self.holding(path).exists(path)
        }

        fn stat(&self, path: &Path) -> io::Result<crate::store::EntryKind> {
            self.note(path);
            self.holding(path).stat(path)
        }

        fn list(
            &self,
            path: &Path,
        ) -> io::Result<Vec<(std::ffi::OsString, crate::store::EntryKind)>> {
            self.note(path);
            self.holding(path).list(path)
        }

        fn remove_dir(&self, path: &Path) -> io::Result<()> {
            self.note(path);
            self.holding(path).remove_dir(path)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.note(path);
            self.holding(path).remove_file(path)
        }
    }

    // ── folder ops, spawners, the registry ───────────────────────────────────────────────────────────────

    /// Folder ops whose stat answers from a table (default `Ok`); a folder's canonical path is its spelling except for
    /// the folders listed in the second table (an OS error), and no folder has an identity (`Unsupported`); nothing
    /// else is modeled. The canonicalize of the folder at the door (the third field) waits until the door opens.
    #[derive(Default)]
    struct StatOps(
        Mutex<HashMap<PathBuf, FolderStatus>>,
        Mutex<Vec<PathBuf>>,
        CanonicalDoor,
    );

    /// A folder whose canonicalize waits (at most 60 s) until [`CanonicalDoor::open`]; `parked` = one is waiting.
    /// `passed` counts every canonicalize.
    #[derive(Default)]
    struct CanonicalDoor {
        at: Mutex<Option<(PathBuf, bool)>>,
        moved: Condvar,
        passed: std::sync::atomic::AtomicUsize,
    }

    impl CanonicalDoor {
        fn close_at(&self, folder: PathBuf) {
            *lock(&self.at) = Some((folder, false));
        }

        fn wait_parked(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut at = lock(&self.at);
            while !at.as_ref().is_some_and(|(_, parked)| *parked) {
                let now = Instant::now();
                assert!(now < deadline, "no canonicalize reached the door");
                at = wait_timeout(&self.moved, at, deadline - now).0;
            }
        }

        fn open(&self) {
            *lock(&self.at) = None;
            self.moved.notify_all();
        }

        fn pass(&self, p: &Path) {
            self.passed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut at = lock(&self.at);
            if let Some((_, parked)) = at.as_mut().filter(|(folder, _)| folder == p) {
                *parked = true;
                self.moved.notify_all();
            }
            while at.as_ref().is_some_and(|(folder, _)| folder == p) {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                at = wait_timeout(&self.moved, at, deadline - now).0;
            }
        }
    }

    impl FolderOps for StatOps {
        fn stat(&self, p: &Path) -> FolderStatus {
            lock(&self.0).get(p).cloned().unwrap_or(FolderStatus::Ok)
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            self.2.pass(p);
            match lock(&self.1).iter().any(|listed| listed == p) {
                true => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                false => Ok(p.to_path_buf()),
            }
        }

        fn identity(&self, _: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
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
            // Taken first: a job that spawns (a bounded stat's worker) must not meet this lock held across the joins.
            let handles = std::mem::take(&mut *lock(&self.0));
            for handle in handles {
                join(handle);
            }
        }
    }

    /// Refuses every spawn — the service's own jobs and the bounded IO's workers alike.
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
            Some(root) => LibraryConfig::usable(state, MountPlace::Explicit(PathBuf::from(root))),
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

    /// A service over two mounts on their own test roots — `library` at `/lib` and `team` at `/team` (and a spare root
    /// `/lib2`) — with no agents, every stat `Ok` and threads for jobs.
    fn two_mounts() -> (Arc<LibraryService>, Arc<TestStore>) {
        mounted(&[("library", "/lib"), ("team", "/team")])
    }

    /// A service over `mounts` (name, folder — each one of the test roots `/lib`, `/team`, `/lib2`), with no agents,
    /// every stat `Ok` and threads for jobs.
    fn mounted(mounts: &[(&str, &str)]) -> (Arc<LibraryService>, Arc<TestStore>) {
        mounted_with(mounts, Arc::new(StatOps::default()))
    }

    /// [`mounted`] whose folder ops are `ops`.
    fn mounted_with(
        mounts: &[(&str, &str)],
        ops: Arc<dyn FolderOps>,
    ) -> (Arc<LibraryService>, Arc<TestStore>) {
        mounted_agents(mounts, ops, Vec::new())
    }

    /// [`mounted_with`] whose registry holds `profiles`.
    fn mounted_agents(
        mounts: &[(&str, &str)],
        ops: Arc<dyn FolderOps>,
        profiles: Vec<AgentProfile>,
    ) -> (Arc<LibraryService>, Arc<TestStore>) {
        mounted_spawn(mounts, ops, profiles, Arc::new(ThreadSpawn))
    }

    /// [`mounted_agents`] whose jobs and bounded IO workers are spawned by `spawn`.
    fn mounted_spawn(
        mounts: &[(&str, &str)],
        ops: Arc<dyn FolderOps>,
        profiles: Vec<AgentProfile>,
        spawn: Arc<dyn Spawn>,
    ) -> (Arc<LibraryService>, Arc<TestStore>) {
        let io = BoundedIo::new(IO_LIMIT, LaunchWaits::LAUNCH, LIST_CAP, ops, spawn);
        mounted_io(mounts, profiles, io)
    }

    /// [`mounted_agents`] over the bounded IO `io`.
    fn mounted_io(
        mounts: &[(&str, &str)],
        profiles: Vec<AgentProfile>,
        io: BoundedIo,
    ) -> (Arc<LibraryService>, Arc<TestStore>) {
        let store = TestStore::new(&["/lib", "/team", "/lib2"]);
        let mount = |(name, folder): &(&str, &str)| MountSetting {
            name: MountName::parse(name).unwrap(),
            place: MountPlace::Explicit(PathBuf::from(folder)),
        };
        let config =
            LibraryConfig::mounted(Path::new("/state"), mounts.iter().map(mount).collect());
        let values = LibraryValues {
            registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles)))),
            backends: Arc::new(AgentBackends),
            config,
            default_root: PathBuf::from("/default-library"),
            daemon_id: "test-daemon".to_owned(),
            root_generation_base: ROOT_BASE,
            builtin_dir: None,
            launch_root: PathBuf::from("/launch"),
            home: None,
            env: Arc::new(|_| None),
        };
        let port: Arc<dyn LibraryStore> = Arc::clone(&store) as Arc<dyn LibraryStore>;
        (Arc::new(LibraryService::new(values, port, io)), store)
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
        let mounts = svc.with_read(|state| state.mounts.clone());
        match svc.observe(&mounts, &lp(rel)) {
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

    /// Relocates the mount `name` to `folder` through the gate's mount change with the service's walk — what
    /// `relocate_mount` publishes, without its normalized-path, stat and overlap checks (the test roots are not absolute
    /// paths on every OS).
    fn relocate(svc: &LibraryService, name: &str, folder: &str) -> Result<RefreshReport, String> {
        let change = MountChange::Relocate {
            name: MountName::parse(name).unwrap(),
            folder: PathBuf::from(folder),
        };
        svc.gate
            .change_mount(change, |mount| svc.build_scan(mount, None, &Tags::new()))
    }

    /// The first mount's folder.
    fn root_of(svc: &LibraryService) -> Option<PathBuf> {
        svc.with_read(|state| state.mounts.iter().next().map(|first| first.folder.clone()))
    }

    /// The published mounts as (name, folder), in order.
    fn mounts_of(svc: &LibraryService) -> Vec<(String, PathBuf)> {
        svc.with_read(|state| {
            state
                .mounts
                .iter()
                .map(|mount| (mount.name.as_str().to_owned(), mount.folder.clone()))
                .collect()
        })
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
            vec![agent(&["library/top.yaml"])],
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let mem = store.mem("/lib");
        let rules = "# Rules\n\nKeep it short.\n";
        mem.file("rules/a.md", rules);
        mem.file(
            "top.yaml",
            "kind: preset\nversion: 1\ndescription: the top\ninstructions:\n  - library/rules/a.md\nskills:\n  - library/skills/s\n",
        );
        mem.file(
            "skills/s/SKILL.md",
            "---\nname: s\ndescription: does s\n---\nbody\n",
        );
        mem.file("broken.yaml", "kind: preset\nversion: [\n");
        mem.file("plain.yaml", "a: 1\n");

        let root = svc.list(Some(&lp("library"))).unwrap();
        assert_eq!(paths(&root), ["library/broken.yaml", "library/top.yaml"]);
        assert_eq!(folder_paths(&root), ["library/rules", "library/skills"]);
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

        let rules_rows = svc.list(Some(&lp("library/rules"))).unwrap();
        assert_eq!(paths(&rules_rows), ["library/rules/a.md"]);
        let a = &rules_rows.rows[0];
        assert_eq!(a.kind, ItemKind::Instruction);
        assert_eq!(a.description, excerpt(rules));
        assert_eq!(a.references, 1, "the preset");

        let skills = svc.list(Some(&lp("library/skills"))).unwrap();
        assert_eq!(paths(&skills), ["library/skills/s"]);
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
        assert_eq!(store.scans(), [scan("/lib", None)], "only the first scan");
    }

    #[test]
    fn list_answers_the_plain_subfolders_directly_in_the_folder() {
        let (svc, store) = library();
        store.mem("/lib").file("a/b/c.md", "c");
        store.mem("/lib").file("a/z/d.md", "d");
        let top = svc.list(None).unwrap();
        assert!(top.rows.is_empty(), "the virtual root holds no item");
        assert_eq!(
            top.folders,
            [
                FolderRow {
                    path: "engram".to_owned(),
                    name: "engram".to_owned(),
                    mount_state: Some(no_builtin_folder()),
                },
                FolderRow {
                    path: "library".to_owned(),
                    name: "library".to_owned(),
                    mount_state: Some(MountState::Ok),
                }
            ],
            "the mounts are its folders, the built-in among them"
        );
        let root = svc.list(Some(&lp("library"))).unwrap();
        assert!(root.rows.is_empty());
        assert_eq!(
            root.folders,
            [FolderRow {
                path: "library/a".to_owned(),
                name: "a".to_owned(),
                mount_state: None,
            }]
        );
        let a = svc.list(Some(&lp("library/a"))).unwrap();
        assert!(a.rows.is_empty());
        assert_eq!(
            folder_paths(&a),
            ["library/a/b", "library/a/z"],
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
                0 => drop(svc.list(Some(&lp("library"))).unwrap()),
                1 => drop(svc.list(Some(&lp("library/a"))).unwrap()),
                2 => drop(svc.refresh(Some(&lp("library/a")))),
                _ => svc.first_scan_join().unwrap(),
            }
            assert_eq!(store.scans(), [scan("/lib", None)], "entrance {entrance}");
            assert!(has(&svc, "library/b/y.md"), "entrance {entrance}");
        }
    }

    #[test]
    fn readers_attach_to_a_walking_first_scan() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(Some(&lp("library"))).unwrap());
        store.wait_held();
        let second = on_thread(&svc, |svc| svc.list(Some(&lp("library"))).unwrap());
        svc.gate
            .wait_for("the second reader waits", |_, waiting| waiting == 1);
        store.release();
        let (first, second) = (join(first), join(second));
        assert_eq!(paths(&first), ["library/a.md"]);
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
        svc.list(Some(&lp("library"))).unwrap();
        mem.file("later.md", "l");
        assert!(
            svc.list(Some(&lp("library/later.md")))
                .unwrap()
                .rows
                .is_empty(),
            "a file has no children"
        );
        assert!(has(&svc, "library/later.md"), "merged by the safety net");

        let through = svc.list(Some(&lp("library/lnk/x.md"))).unwrap();
        assert_eq!(paths(&through), ["library/lnk/x.md"]);
        assert_eq!(
            through.rows[0].state.as_ref().unwrap().code,
            PATH_THROUGH_LINK
        );
        assert!(!has(&svc, "library/lnk/x.md"));
        for name in [".hidden.md", "x.md.tmp12.0"] {
            assert!(svc.list(Some(&lp(name))).unwrap().rows.is_empty(), "{name}");
            assert!(!has(&svc, name), "{name}");
        }
        // The same admission gates every late merge's read.
        let root = &svc.with_read(|state| state.mounts.clone());
        assert!(matches!(
            svc.observe(root, &lp("library/lnk/x.md")),
            Observed::ThroughLink
        ));
        assert!(matches!(
            svc.observe(root, &lp("library/.hidden.md")),
            Observed::Refused
        ));
        assert!(matches!(
            svc.observe(root, &lp("library/x.md.tmp12.0")),
            Observed::Refused
        ));
    }

    // ── mounts: the virtual tree over several folders ──────────────────────────────────────────────────────

    // ADR-0396 · ADR-0400 decision 4: the virtual root lists the mounts; each mount's items sit under its name, keyed
    //   by the virtual path, so one spelling in two mounts is two items.
    #[test]
    fn a_whole_scan_walks_every_mount_and_the_catalog_holds_each_under_its_name() {
        let (svc, store) = two_mounts();
        store.mem("/lib").file("rules/a.md", "mine");
        store.mem("/team").file("rules/a.md", "theirs");
        let top = svc.list(None).unwrap();
        assert!(top.rows.is_empty());
        assert_eq!(folder_paths(&top), ["engram", "library", "team"]);
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/team", None)],
            "one whole scan, every mount"
        );
        assert_eq!(
            paths(&svc.list(Some(&lp("team/rules"))).unwrap()),
            ["team/rules/a.md"]
        );
        let excerpt_of = |path: &str| {
            svc.with_read(
                |state| match &state.catalog.get(&key(&lp(path))).unwrap().content {
                    ItemContent::Instruction { excerpt } => excerpt.clone(),
                    other => panic!("{other:?}"),
                },
            )
        };
        assert_eq!(
            (
                excerpt_of("library/rules/a.md"),
                excerpt_of("team/rules/a.md")
            ),
            ("mine".to_owned(), "theirs".to_owned())
        );
        let read = svc.read(&lp("team/rules/a.md")).unwrap();
        assert_eq!(read.absolute, Path::new("/team").join("rules").join("a.md"));
    }

    // ADR-0400 decisions 4, 6 and 17: a mount that is not there is not walked and the other one still reads; the
    //   virtual root carries each mount's last known state, whole-tree reads answer partially and name it, a read
    //   inside it refuses naming it, and once it is back its next reader walks it.
    #[test]
    fn a_mount_that_is_not_there_is_left_out_named_and_walked_again_by_its_next_reader_once_back() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(PathBuf::from("/team"), FolderStatus::Missing);
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
        );
        store.mem("/lib").file("rules/a.md", "mine");
        store.mem("/team").file("rules/b.md", "theirs");
        let team = MountName::parse("team").unwrap();
        let missing = MountState::Missing {
            why: "missing".to_owned(),
        };
        let states = |answer: &ListAnswer| -> Vec<(String, Option<MountState>)> {
            answer
                .folders
                .iter()
                .map(|row| (row.path.clone(), row.mount_state.clone()))
                .collect()
        };
        let top = svc.list(None).unwrap();
        assert_eq!(
            states(&top),
            [
                ("engram".to_owned(), Some(no_builtin_folder())),
                ("library".to_owned(), Some(MountState::Ok)),
                ("team".to_owned(), Some(missing.clone()))
            ]
        );
        assert!(top.unavailable.is_empty(), "the rows carry the state");
        assert_eq!(
            store.scans(),
            [scan("/lib", None)],
            "the mount that is not there is not walked"
        );
        assert_eq!(
            paths(&svc.list(Some(&lp("library/rules"))).unwrap()),
            ["library/rules/a.md"]
        );

        let builtin = UnavailableMount {
            mount: BUILTIN_MOUNT.to_owned(),
            state: no_builtin_folder(),
        };
        let unavailable = [
            builtin.clone(),
            UnavailableMount {
                mount: team.as_str().to_owned(),
                state: missing.clone(),
            },
        ];
        let query = ListFilter {
            query: Some(".md".to_owned()),
            ..ListFilter::default()
        };
        let found = svc.list_with(None, &query).unwrap();
        assert_eq!(paths(&found), ["library/rules/a.md"]);
        assert_eq!(found.unavailable, unavailable);
        let folders = svc.folders().unwrap();
        assert_eq!(folders.unavailable, unavailable);
        assert!(
            folders
                .folders
                .iter()
                .all(|folder| !folder.path.starts_with("team/")),
            "{folders:?}"
        );

        let (class, reason) = refused(svc.read(&lp("team/rules/b.md")).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (
                RefusalClass::Unavailable,
                LIBRARY_UNAVAILABLE,
                &json!("team")
            )
        );
        let listed = svc.list(Some(&lp("team/rules"))).unwrap_err();
        assert_eq!(
            (listed.code.as_str(), &listed.args["mount"]),
            (LIBRARY_UNAVAILABLE, &json!("team"))
        );
        assert_eq!(
            store.scans().len(),
            1,
            "stat'ed again, still not there: not walked"
        );

        lock(&ops.0).clear();
        assert_eq!(
            paths(&svc.list(Some(&lp("team/rules"))).unwrap()),
            ["team/rules/b.md"],
            "its next reader walked it"
        );
        assert_eq!(store.scans(), [scan("/lib", None), scan("/team", None)]);
        assert_eq!(
            states(&svc.list(None).unwrap())[2],
            ("team".to_owned(), Some(MountState::Ok))
        );
        assert_eq!(svc.folders().unwrap().unavailable, [builtin]);
    }

    // ADR-0387 per mount · ADR-0400 decision 17: a read entrance stats the mounts its presets name; one that is not
    //   there or cannot be read is block ④ by its stat alone, and one that is back is walked again before it is judged.
    #[test]
    fn a_read_entrance_walks_a_named_mount_again_once_it_is_back() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
        );
        store.mem("/team").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/a.md\n",
        );
        store.mem("/team").file("a.md", "a");
        let down = svc.resolve_preset(&lp("team/P.yaml"));
        let [BlockReason::LibraryUnavailable { reason }] = down.problems.as_slice() else {
            panic!("{:?}", down.problems)
        };
        assert_eq!(reason.args["message"], json!("down"));
        assert!(down.blocked().is_some(), "there but unreadable: blocked");
        assert_eq!(store.scans(), [scan("/lib", None)], "not walked while down");

        lock(&ops.0).clear();
        let back = svc.resolve_preset(&lp("team/P.yaml"));
        assert!(back.problems.is_empty(), "{:?}", back.problems);
        assert_eq!(
            back.instructions,
            [("team/a.md".to_owned(), Path::new("/team").join("a.md"))]
        );
        assert_eq!(store.scans(), [scan("/lib", None), scan("/team", None)]);
    }

    /// What a refusal naming mount `team` as not readable answers: its class, code and `mount`.
    fn team_unreadable() -> (RefusalClass, String, serde_json::Value) {
        (
            RefusalClass::Unavailable,
            LIBRARY_UNAVAILABLE.to_owned(),
            json!("team"),
        )
    }

    fn class_code_mount(refusal: LibraryRefusal) -> (RefusalClass, String, serde_json::Value) {
        let (class, reason) = refused(refusal);
        (class, reason.code, reason.args["mount"].clone())
    }

    // ADR-0400 decision 6: a write into a mount that cannot be read, and `preset.get` of one of its presets, refuse
    //   naming the mount before any change — its disk is not touched under the write lock, and no stale catalog row
    //   answers.
    #[test]
    fn writes_and_preset_get_in_a_mount_that_cannot_be_read_refuse_naming_it_and_touch_nothing() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        let (svc, store) = mounted_with(&[("library", "/lib"), ("team", "/team")], ops);
        let preset = "kind: preset\nversion: 1\ninstructions: []\n";
        store.mem("/team").file("P.yaml", preset);
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let p = lp("team/P.yaml");
        assert_eq!(
            class_code_mount(svc.preset_get(&AssetRef::Library(p.clone())).unwrap_err()),
            team_unreadable(),
            "preset.get"
        );
        let add = edit::AddEntries::Items(vec![edit::NewItem {
            path: "library/a.md".to_owned(),
            backend: None,
        }]);
        assert_eq!(
            class_code_mount(svc.preset_add(&p, &add, None, None).unwrap_err()),
            team_unreadable(),
            "an edit"
        );
        let markdown = create::NewFileKind::Markdown;
        assert_eq!(
            class_code_mount(svc.new_file(Some("team"), "x", markdown, None).unwrap_err()),
            team_unreadable(),
            "newFile"
        );
        assert_eq!(
            class_code_mount(svc.new_folder(Some("team"), "d", None).unwrap_err()),
            team_unreadable(),
            "newFolder"
        );
        assert!(
            store.touched_under("/team").is_empty(),
            "no probe, read or write of the mount: {:?}",
            store.touched_under("/team")
        );
        let team = Path::new("/team");
        assert_eq!(
            store.read(&team.join("P.yaml"), u64::MAX).unwrap(),
            preset.as_bytes()
        );
        assert!(!store.exists(&team.join("x.md")) && !store.exists(&team.join("d")));
        assert_eq!(store.scans(), [scan("/lib", None)], "never walked");
    }

    // ADR-0400 decision 6: a relink rewrites no file of a mount that cannot be read — each is skipped naming the mount
    //   — and refuses a target in one.
    #[test]
    fn a_relink_skips_the_files_of_a_mount_that_cannot_be_read_and_refuses_a_target_in_one() {
        let ops = Arc::new(StatOps::default());
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
        );
        let lists_old = "kind: preset\nversion: 1\ninstructions:\n  - library/old.md\n";
        store.mem("/lib").file("P.yaml", lists_old);
        store.mem("/lib").file("new.md", "new");
        store.mem("/team").file("Q.yaml", lists_old);
        store.mem("/team").file("new.md", "new");
        svc.first_scan_join().unwrap();
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        assert_eq!(svc.refresh(Some(&lp("team"))).errors.len(), 1);

        assert_eq!(
            class_code_mount(
                svc.relink("library/old.md", "team/new.md", None)
                    .unwrap_err()
            ),
            team_unreadable(),
            "a target in it"
        );
        let answer = svc
            .relink("library/old.md", "library/new.md", None)
            .unwrap();
        assert_eq!(answer.written, ["library/P.yaml"]);
        let [skip] = answer.skipped.as_slice() else {
            panic!("{:?}", answer.skipped)
        };
        assert_eq!(
            (
                skip.path.as_str(),
                skip.reason.code.as_str(),
                &skip.reason.args["mount"]
            ),
            ("team/Q.yaml", LIBRARY_UNAVAILABLE, &json!("team"))
        );
        assert!(
            answer.fixed.is_empty(),
            "a file listing the old path is left"
        );
        assert_eq!(
            store
                .read(&Path::new("/team").join("Q.yaml"), u64::MAX)
                .unwrap(),
            lists_old.as_bytes()
        );
    }

    // ADR-0400 decision 17: a read into a mount that is not there stats it fresh and refuses without a ticket — it
    //   does not queue behind a slow walk of another mount.
    #[test]
    fn a_read_into_a_mount_that_is_not_there_does_not_queue_behind_a_slow_walk() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(PathBuf::from("/team"), FolderStatus::Missing);
        let (svc, store) = mounted_with(&[("library", "/lib"), ("team", "/team")], ops);
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        store.hold_at(At::Start, "/lib");
        let slow = on_thread(&svc, |svc| svc.refresh(Some(&lp("library"))));
        store.wait_held();
        // Joined while the walk is held: a read that drew a ticket would wait behind it and hang here.
        let read = on_thread(&svc, |svc| svc.read(&lp("team/x.md")).map(drop));
        assert_eq!(class_code_mount(join(read).unwrap_err()), team_unreadable());
        store.release();
        assert!(join(slow).errors.is_empty());
    }

    // ADR-0400 decision 17 · ADR-0387 per mount: a read entrance judges a mount walked before whose walk again failed
    //   by that failure — the rule `list` and `read` use — not from the catalog the earlier walk left.
    #[test]
    fn a_read_entrance_blocks_on_a_mount_whose_walk_again_failed_though_it_was_walked_before() {
        let ops = Arc::new(StatOps::default());
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
        );
        store.mem("/team").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/a.md\n",
        );
        store.mem("/team").file("a.md", "a");
        svc.first_scan_join().unwrap();
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        svc.refresh(Some(&lp("team")));
        lock(&ops.0).clear();
        *lock(&store.fail_next_scan) = true;
        let resolution = svc.resolve_preset(&lp("team/P.yaml"));
        let [BlockReason::LibraryUnavailable { reason }] = resolution.problems.as_slice() else {
            panic!("{:?}", resolution.problems)
        };
        assert_eq!(
            reason.args["message"],
            json!(io::Error::from(io::ErrorKind::PermissionDenied).to_string())
        );
        assert!(resolution.blocked().is_some());
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/team", None), scan("/team", None)],
            "walked again once, and that walk failed"
        );
    }

    // ADR-0387 per mount: a mount that stats `Ok` at the entrance but is gone when walked again is absent — skipped,
    //   not blocked.
    #[test]
    fn a_read_entrance_skips_a_mount_found_gone_when_walked_again() {
        let ops = Arc::new(CountOps::default());
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
        );
        store.mem("/team").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/a.md\n",
        );
        store.mem("/team").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let team = PathBuf::from("/team");
        lock(&ops.later).insert(team.clone(), FolderStatus::Unreadable("down".to_owned()));
        svc.refresh(Some(&lp("team")));
        lock(&ops.later).clear();
        // The entrance's own stat finds it; the fresh stat before its walk again does not.
        *lock(&ops.from) = Some((team.clone(), ops.stats_of(&team) + 2, FolderStatus::Missing));
        let resolution = svc.resolve_preset(&lp("team/P.yaml"));
        assert_eq!(
            resolution.absent_mounts,
            absent("team", "missing"),
            "{:?}",
            resolution.problems
        );
        assert_eq!(
            resolution.problems,
            [library_unavailable("team", "missing")]
        );
        assert!(resolution.blocked().is_none());
    }

    // ADR-0400 decision 19: a read entrance stats the mount a library preset's reference reaches, though that mount's
    //   last walk was `Ok`: its fresh stat is the answer — one that cannot be read blocks with its ④ row, one that is
    //   gone is a skipped ④ row — and the mount is stat'ed once.
    #[test]
    fn a_read_entrance_judges_a_walked_mount_its_presets_reach_by_its_fresh_stat() {
        for (status, blocks) in [
            (FolderStatus::Unreadable("down".to_owned()), true),
            (FolderStatus::Missing, false),
        ] {
            let ops = Arc::new(CountOps::default());
            let (svc, store) = mounted_with(
                &[("library", "/lib"), ("team", "/team")],
                Arc::clone(&ops) as Arc<dyn FolderOps>,
            );
            store.mem("/lib").file(
                "P.yaml",
                "kind: preset\nversion: 1\ninstructions:\n  - team/a.md\n",
            );
            store.mem("/team").file("a.md", "a");
            svc.first_scan_join().unwrap();
            let team = PathBuf::from("/team");
            assert!(svc.with_read(|state| state.seen(&MountName::parse("team").unwrap()).usable()));
            let before = ops.stats_of(&team);
            lock(&ops.later).insert(team.clone(), status.clone());
            let resolution = svc.resolve_preset(&lp("library/P.yaml"));
            assert_eq!(
                resolution.problems,
                [library_unavailable("team", &status.why())],
                "{status:?}"
            );
            assert_eq!(resolution.blocked().is_some(), blocks, "{status:?}");
            assert_eq!(
                ops.stats_of(&team) - before,
                1,
                "{status:?}: one stat of /team"
            );
        }
    }

    // ADR-0400 decision 19: a read entrance follows references by its want's tag rule, as resolution takes them — a
    //   mount only another backend's entry reaches is not stat'ed for the agent, so its dead share costs that agent's
    //   folder stats nothing; one preset on its own takes every tag and stats it.
    #[test]
    fn a_read_entrance_does_not_stat_a_mount_only_another_backends_entry_reaches() {
        let codex = coded(
            AgentCommand::Codex {
                extra_args: Vec::new(),
                output_format: AgentOutputFormat::StreamJson,
            },
            &["library/P.yaml"],
        );
        let ops = Arc::new(CountOps::default());
        let team = PathBuf::from("/team");
        lock(&ops.later).insert(team.clone(), FolderStatus::Unreadable("down".to_owned()));
        let (svc, store) = mounted_agents(
            &[("library", "/lib"), ("team", "/team")],
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            vec![codex.clone()],
        );
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/a.md\n  - {path: team/a.md, backend: claude}\n",
        );
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let before = ops.stats_of(&team);
        let resolution = svc.resolve_agent(codex.id).expect("a codex agent");
        assert_eq!(
            ops.stats_of(&team),
            before,
            "the agent's entrance stats no team"
        );
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert_eq!(resolution.instructions[0].0, "library/a.md");

        let alone = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            ops.stats_of(&team),
            before + 1,
            "a preset alone takes every tag"
        );
        assert_eq!(alone.problems, [library_unavailable("team", "down")]);
    }

    // ADR-0400 decision 19: a `preset.get` row in a mount that cannot be used is not looked up in a catalog the mount
    //   cannot vouch for — one that cannot be read answers `library-unavailable` with the mount, an absent one
    //   `not-found` — and no row in it is link-probed: nothing in its folder is touched.
    #[test]
    fn preset_get_rows_in_a_mount_that_cannot_be_used_are_not_looked_up_or_probed() {
        for (status, want) in [
            (
                FolderStatus::Unreadable("down".to_owned()),
                json!({ "code": LIBRARY_UNAVAILABLE, "args": { "mount": "team", "message": "down" } }),
            ),
            (FolderStatus::Missing, json!({ "code": NOT_FOUND })),
        ] {
            let ops = Arc::new(StatOps::default());
            let (svc, store) = mounted_with(
                &[("library", "/lib"), ("team", "/team")],
                Arc::clone(&ops) as Arc<dyn FolderOps>,
            );
            store.mem("/lib").file(
                "P.yaml",
                "kind: preset\nversion: 1\ninstructions:\n  - team/a.md\n  - team/x.md\n",
            );
            store.mem("/team").file("a.md", "a");
            svc.first_scan_join().unwrap();
            lock(&ops.0).insert(PathBuf::from("/team"), status.clone());
            lock(&store.touched).clear();
            let view = svc
                .preset_get(&AssetRef::Library(lp("library/P.yaml")))
                .unwrap();
            for row in &view.instructions {
                let state = row
                    .state
                    .as_ref()
                    .expect("a row in a mount that cannot be used");
                assert_eq!(state.code, want["code"], "{status:?} {}", row.path);
                if let Some(args) = want.get("args") {
                    assert_eq!(&state.args, args, "{status:?} {}", row.path);
                }
            }
            assert!(
                store.touched_under("/team").is_empty(),
                "{status:?}: {:?}",
                store.touched_under("/team")
            );
        }
    }

    // ADR-0400 decision 6: a relink cannot know what a mount never walked refers to — the mount is one skipped row at
    //   its top, beside the pairs fixed within the mounts it could read.
    #[test]
    fn a_relink_reports_a_mount_never_walked_beside_the_pairs_fixed_in_the_mounts_read() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        let (svc, store) = mounted_with(&[("library", "/lib"), ("team", "/team")], ops);
        let lists_old = "kind: preset\nversion: 1\ninstructions:\n  - library/old.md\n";
        store.mem("/lib").file("P.yaml", lists_old);
        store.mem("/lib").file("new.md", "new");
        store.mem("/team").file("Q.yaml", lists_old);
        svc.first_scan_join().unwrap();
        let answer = svc
            .relink("library/old.md", "library/new.md", None)
            .unwrap();
        assert_eq!(answer.written, ["library/P.yaml"]);
        let [skip] = answer.skipped.as_slice() else {
            panic!("{:?}", answer.skipped)
        };
        assert_eq!(
            (
                skip.path.as_str(),
                skip.reason.code.as_str(),
                &skip.reason.args["mount"]
            ),
            ("team", LIBRARY_UNAVAILABLE, &json!("team"))
        );
        assert_eq!(
            answer.fixed,
            [("library/old.md".to_owned(), "library/new.md".to_owned())]
        );
        assert_eq!(answer.more, 0);
        assert!(store.touched_under("/team").is_empty());
    }

    // ADR-0400 decision 6: a write queued behind a scan that finds its mount gone after the entrance passed it judges
    //   the mount again under the write lock, before any disk IO — it refuses (a relink skips the mount's files), and
    //   the mount's disk is not touched.
    #[test]
    fn writes_queued_behind_a_scan_that_finds_their_mount_gone_refuse_before_any_disk_io() {
        let (svc, store) = mounted_with(
            &[("library", "/lib"), ("team", "/team")],
            Arc::new(StatOps::default()),
        );
        let lists_old = "kind: preset\nversion: 1\ninstructions:\n  - library/old.md\n";
        store.mem("/lib").file("P.yaml", lists_old);
        store.mem("/lib").file("a.md", "a");
        store.mem("/lib").file("new.md", "new");
        store.mem("/team").file("Q.yaml", lists_old);
        let empty = "kind: preset\nversion: 1\ninstructions: []\n";
        store.mem("/team").file("E.yaml", empty);
        svc.first_scan_join().unwrap();
        lock(&store.touched).clear();

        store.hold_at(At::Start, "/lib");
        let slow = on_thread(&svc, |svc| svc.refresh(Some(&lp("library"))));
        store.wait_held();
        let edit = on_thread(&svc, |svc| {
            let add = edit::AddEntries::Items(vec![edit::NewItem {
                path: "library/a.md".to_owned(),
                backend: None,
            }]);
            svc.preset_add(&lp("team/E.yaml"), &add, None, None)
                .map(drop)
        });
        let create = on_thread(&svc, |svc| {
            svc.new_file(Some("team"), "x", create::NewFileKind::Markdown, None)
                .map(drop)
        });
        let relink = on_thread(&svc, |svc| {
            svc.relink("library/old.md", "library/new.md", None)
        });
        svc.gate
            .wait_for("the three writes wait behind the walk", |line, _| {
                line == [
                    ("scan", true),
                    ("change", false),
                    ("change", false),
                    ("change", false),
                ]
            });
        // The entrances passed; now the mount is found gone, recorded without a ticket.
        let team = svc.with_read(|state| state.mounts.get("team").cloned().unwrap());
        svc.gate.observe(vec![team], &|mounts| {
            vec![Some(FolderStatus::Missing); mounts.len()]
        });
        store.release();
        assert!(join(slow).errors.is_empty());
        assert_eq!(class_code_mount(join(edit).unwrap_err()), team_unreadable());
        assert_eq!(
            class_code_mount(join(create).unwrap_err()),
            team_unreadable()
        );
        let answer = join(relink).unwrap();
        assert_eq!(answer.written, ["library/P.yaml"]);
        assert_eq!(
            answer
                .skipped
                .iter()
                .map(|skip| (skip.path.as_str(), skip.reason.code.as_str()))
                .collect::<Vec<_>>(),
            [("team/Q.yaml", LIBRARY_UNAVAILABLE)]
        );
        assert!(
            store.touched_under("/team").is_empty(),
            "{:?}",
            store.touched_under("/team")
        );
    }

    // ADR-0400 decision 6: `preset.check` of a preset in a mount that cannot be read refuses naming it, as `preset.get`
    //   does — its current lists would come from a stale catalog.
    #[test]
    fn a_preset_check_in_a_mount_that_cannot_be_read_refuses_naming_it() {
        let ops = Arc::new(StatOps::default());
        lock(&ops.0).insert(
            PathBuf::from("/team"),
            FolderStatus::Unreadable("down".to_owned()),
        );
        let (svc, store) = mounted_with(&[("library", "/lib"), ("team", "/team")], ops);
        store
            .mem("/team")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        svc.first_scan_join().unwrap();
        let draft = edit::PresetDraft::default();
        assert_eq!(
            class_code_mount(svc.preset_check(&lp("team/P.yaml"), &draft).unwrap_err()),
            team_unreadable()
        );
    }

    // ADR-0400 decision 4: a mount's name is a folder, not an item; a first name that names no mount reads as absent.
    #[test]
    fn a_mount_name_or_a_path_in_no_mount_reads_as_not_found() {
        let (svc, store) = two_mounts();
        store.mem("/lib").file("a.md", "a");
        for path in ["library", "team", "nope/a.md", "a.md"] {
            let (class, reason) = refused(svc.read(&lp(path)).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::NotFound, NOT_FOUND),
                "{path}"
            );
        }
        let nowhere = svc.list(Some(&lp("nope"))).unwrap();
        assert!(nowhere.rows.is_empty() && nowhere.folders.is_empty());
        assert!(has(&svc, "library/a.md"));
    }

    // ADR-0396 decision 3: a whole scan reports a reference into another mount as missing only when it is.
    #[test]
    fn a_whole_scan_reports_no_reference_into_another_mount_as_missing() {
        let (svc, store) = two_mounts();
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/rules/a.md\n  - team/gone.md\n",
        );
        store.mem("/team").file("rules/a.md", "a");
        let report = svc.refresh(None);
        let missing: Vec<(&str, &str)> = report
            .problems
            .iter()
            .filter(|(_, reason)| reason.code == NOT_FOUND)
            .map(|(preset, reason)| (preset.as_str(), reason.args["path"].as_str().unwrap()))
            .collect();
        assert_eq!(missing, [("library/P.yaml", "team/gone.md")]);
    }

    // A refresh walks only the mount its folder lies in; one in no mount walks nothing and says why.
    #[test]
    fn a_folder_refresh_walks_only_its_mount_and_one_in_no_mount_answers_why() {
        let (svc, store) = two_mounts();
        store.mem("/team").file("rules/a.md", "a");
        svc.first_scan_join().unwrap();
        store.mem("/team").file("rules/b.md", "b");
        let report = svc.refresh(Some(&lp("team/rules")));
        assert_eq!(report.changed, 1, "{report:?}");
        assert!(has(&svc, "team/rules/b.md"));
        assert_eq!(
            store.scans()[2..],
            [scan("/team", Some("rules"))],
            "only the team mount's folder"
        );
        let nowhere = svc.refresh(Some(&lp("nope/rules")));
        assert_eq!(nowhere.errors.len(), 1, "{nowhere:?}");
        assert!(nowhere.errors[0].1.contains("nope"), "{nowhere:?}");
        assert_eq!(store.scans().len(), 3, "nothing walked");
    }

    // ADR-0366 as ADR-0400 decision 16 amends it: relocating a mount replaces only its range; the others' items stay.
    #[test]
    fn a_mount_relocation_replaces_only_the_relocated_mounts_range() {
        let (svc, store) = two_mounts();
        store.mem("/lib").file("old.md", "old");
        store.mem("/team").file("t.md", "t");
        store.mem("/lib2").file("new.md", "new");
        svc.first_scan_join().unwrap();
        relocate(&svc, "library", "/lib2").unwrap();
        assert!(!has(&svc, "library/old.md"), "the old folder's items went");
        assert!(has(&svc, "library/new.md"));
        assert!(has(&svc, "team/t.md"), "the other mount's items stay");
        assert_eq!(
            mounts_of(&svc),
            [
                ("library".to_owned(), PathBuf::from("/lib2")),
                ("team".to_owned(), PathBuf::from("/team"))
            ]
        );
    }

    // ADR-0396 decision 3: a reference may name any mount; it resolves to the file in that mount's folder.
    #[test]
    fn a_reference_across_mounts_resolves_to_the_other_mounts_folder() {
        let (svc, store) = two_mounts();
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/rules/a.md\n  - team/gone.md\n",
        );
        store.mem("/team").file("rules/a.md", "a");
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            resolution.instructions,
            [(
                "team/rules/a.md".to_owned(),
                Path::new("/team").join("rules").join("a.md")
            )]
        );
        assert_eq!(codes(&resolution), [NOT_FOUND], "team/gone.md is missing");
    }

    // ADR-0400 decision 2: a valid mount name shaped like a temp file is held by the name rules, not the store's name
    //   rule, so its items are edited and created like any mount's (the launch side: `launch.rs` tests).
    #[test]
    fn a_mount_named_like_a_temp_file_is_edited_and_created_in_like_any_mount() {
        use crate::create::NewFileKind;
        use crate::edit::{AddEntries, NewItem};
        let name = "work.tmp1.2";
        assert!(
            engram_dashboard_base::file::is_temp_name(name),
            "the fixture is temp-shaped"
        );
        let (svc, store) = mounted(&[(name, "/lib")]);
        store
            .mem("/lib")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        store.mem("/lib").file("rules/a.md", "a");
        svc.first_scan_join().unwrap();
        assert!(has(&svc, "work.tmp1.2/P.yaml") && has(&svc, "work.tmp1.2/rules/a.md"));

        let added = svc
            .preset_add(
                &lp("work.tmp1.2/P.yaml"),
                &AddEntries::Items(vec![NewItem {
                    path: "work.tmp1.2/rules/a.md".to_owned(),
                    backend: None,
                }]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(added.applied, ["work.tmp1.2/rules/a.md"]);

        let made = svc
            .new_file(Some("work.tmp1.2/rules"), "c", NewFileKind::Markdown, None)
            .unwrap();
        assert_eq!(made.as_str(), "work.tmp1.2/rules/c.md");
        assert!(has(&svc, "work.tmp1.2/rules/c.md"));
        let (class, reason) = refused(
            svc.new_folder(Some("work.tmp1.2/rules"), "x.tmp1.2", None)
                .unwrap_err(),
        );
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID),
            "a temp name inside the mount is still refused"
        );
    }

    // ADR-0400 decision 5: a write whose target has one name is refused before any mount lookup or kind check.
    #[test]
    fn every_write_to_a_one_name_target_refuses_virtual_root_read_only_before_any_other_check() {
        use crate::edit::{AddEntries, NewItem, RemoveEntries};
        use crate::format::ListName;
        let (svc, store) = two_mounts();
        store
            .mem("/lib")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let item = || {
            AddEntries::Items(vec![NewItem {
                path: "library/a.md".to_owned(),
                backend: None,
            }])
        };
        let want = (RefusalClass::InvalidArgument, VIRTUAL_ROOT_READ_ONLY);
        // `library` is a mount's top (and no `.yaml`); `P.yaml` names no mount; `.x` would be a dot name.
        for target in ["library", "P.yaml", "team", ".x"] {
            let target = lp(target);
            let answers = [
                svc.preset_add(&target, &item(), None, None).map(drop),
                svc.preset_add(
                    &target,
                    &AddEntries::Folders(vec!["C:/x".to_owned()]),
                    None,
                    None,
                )
                .map(drop),
                svc.preset_remove(&target, &RemoveEntries::Items(Vec::new()), None)
                    .map(drop),
                svc.preset_reorder(&target, ListName::Instructions, &[], "rev", None)
                    .map(drop),
                svc.preset_set_backend(&target, &[], None, None).map(drop),
            ];
            for (at, answer) in answers.into_iter().enumerate() {
                let (class, reason) = refused(answer.unwrap_err());
                assert_eq!(
                    (class, reason.code.as_str()),
                    want,
                    "{} write {at}",
                    target.as_str()
                );
                assert_eq!(reason.args["path"], json!(target.as_str()));
            }
        }
        let (class, reason) = refused(svc.relink("library/a.md", "library", None).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            want,
            "relink to a mount's top"
        );
        let (class, reason) = refused(svc.relink("library/a.md", "a.md", None).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            want,
            "relink to a name at the virtual root"
        );
        let unread = String::from_utf8(
            store
                .mem("/lib")
                .read(&Path::new("/lib").join("P.yaml"), u64::MAX)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(unread, "kind: preset\nversion: 1\n", "nothing was written");
    }

    // ADR-0396 decision 3: an edit, a create and a relink in one mount may name another mount's items.
    #[test]
    fn edits_creates_and_relinks_work_across_mounts() {
        use crate::create::NewFileKind;
        use crate::edit::{AddEntries, NewItem};
        let (svc, store) = two_mounts();
        store
            .mem("/lib")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        store.mem("/team").file("rules/a.md", "a");
        store.mem("/team").file("rules/b.md", "b");
        svc.first_scan_join().unwrap();

        let item = |path: &str| NewItem {
            path: path.to_owned(),
            backend: None,
        };
        let added = svc
            .preset_add(
                &lp("library/P.yaml"),
                &AddEntries::Items(vec![item("team/rules/a.md")]),
                None,
                None,
            )
            .unwrap();
        assert_eq!(added.applied, ["team/rules/a.md"]);
        let written = String::from_utf8(
            store
                .mem("/lib")
                .read(&Path::new("/lib").join("P.yaml"), u64::MAX)
                .unwrap(),
        )
        .unwrap();
        assert!(written.contains("- team/rules/a.md"), "{written}");

        let made = svc
            .new_file(Some("team/rules"), "c", NewFileKind::Markdown, None)
            .unwrap();
        assert_eq!(made.as_str(), "team/rules/c.md");
        assert!(store
            .mem("/team")
            .exists(&Path::new("/team").join("rules").join("c.md")));
        assert!(has(&svc, "team/rules/c.md"));

        let relinked = svc
            .relink("team/rules/a.md", "team/rules/b.md", None)
            .unwrap();
        assert_eq!(
            relinked.fixed,
            [("team/rules/a.md".to_owned(), "team/rules/b.md".to_owned())]
        );
        assert_eq!(relinked.written, ["library/P.yaml"]);
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            resolution.instructions,
            [(
                "team/rules/b.md".to_owned(),
                Path::new("/team").join("rules").join("b.md")
            )]
        );
    }

    // ── refresh ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_folder_refresh_before_the_first_scan_answers_its_share_of_a_whole_scan() {
        let (svc, store) = library();
        let mem = store.mem("/lib");
        mem.file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/b/x.md\n  - library/a/missing.md\n",
        );
        mem.file("b/x.md", "x");
        mem.file("b/broken.yaml", "kind: preset\nversion: [\n");
        let report = svc.refresh(Some(&lp("library/a")));
        assert_eq!(store.scans(), [scan("/lib", None)]);
        let missing = Reason {
            code: NOT_FOUND.to_owned(),
            args: json!({ "path": "library/a/missing.md" }),
            line: None,
        };
        // `b/x.md` is found (the whole library was scanned) and `b/broken.yaml` is outside `a`.
        assert_eq!(report.problems, [(lp("library/a/p.yaml"), missing)]);
        assert_eq!(report.changed, 1);
    }

    #[test]
    fn a_folder_refresh_arriving_at_a_walking_first_scan_scans_its_folder_after_it() {
        let (svc, store) = library();
        store.mem("/lib").file("a/x.md", "x");
        store.hold_at(At::Start, "/lib");
        let first = on_thread(&svc, |svc| svc.list(Some(&lp("library"))).unwrap());
        store.wait_held();
        let refresh = on_thread(&svc, |svc| svc.refresh(Some(&lp("library/a"))));
        svc.gate
            .wait_for("the refresh waits for the first scan", |_, waiting| {
                waiting == 1
            });
        store.release();
        join(first);
        join(refresh);
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", Some("a"))]);
    }

    // ADR-0400 decision 17: the whole first scan raises the first scan even when a mount cannot be walked; a folder
    //   refresh in that mount walks the whole mount (decision 6 per mount), so no false not-found is reported.
    #[test]
    fn a_folder_refresh_in_a_mount_the_first_scan_could_not_walk_walks_the_whole_mount() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "a/p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/b/x.md\n",
        );
        store.mem("/lib").file("b/x.md", "x");
        store.mem("/lib").file("c.md", "c");
        *lock(&store.fail_next_scan) = true;
        svc.first_scan_join()
            .expect("a mount it cannot walk is not the scan's failure");
        assert_eq!(
            svc.with_read(|state| state.seen(&MountName::library())),
            MountSeen {
                state: MountState::Unreadable {
                    why: io::Error::from(io::ErrorKind::PermissionDenied).to_string()
                },
                walked: false
            }
        );
        let report = svc.refresh(Some(&lp("library/a")));
        assert_eq!(store.scans(), [scan("/lib", None), scan("/lib", None)]);
        assert!(report.problems.is_empty(), "no false not-found: {report:?}");
        assert!(report.errors.is_empty());
        assert_eq!(report.changed, 1);
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/c.md"],
            "the whole mount was applied"
        );
        assert_eq!(
            store.scans().len(),
            2,
            "the mount is walked: no reader walks it again"
        );
    }

    #[test]
    fn a_refresh_arriving_at_a_walking_covering_scan_scans_again_and_answers_the_new_bytes() {
        let (svc, store) = library();
        store.mem("/lib").file_version("a/x.md", "old", 1);
        svc.list(Some(&lp("library"))).unwrap();
        store.hold_at(At::End, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("a/x.md", "new", 2);
        let late = on_thread(&svc, |svc| svc.refresh(Some(&lp("library/a"))));
        svc.gate
            .wait_for("the late refresh has its own ticket", |line, _| {
                line == [("scan", true), ("scan", false)]
            });
        store.release();
        assert_eq!(join(walking).changed, 0);
        assert_eq!(join(late).changed, 1);
        assert_eq!(version(&svc, "library/a/x.md"), 2);
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
        assert_eq!(report.skipped_links, [lp("library/lnk")]);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert_eq!(report.errors[0].0, Some(lp("library/sub")));
        assert_eq!(report.no_kind, [lp("library/plain.yaml")]);
        let caps = lock(&store.read_caps).clone();
        assert!(!caps.is_empty());
        assert!(caps.iter().all(|&cap| cap == LIBRARY_READ_CAP), "{caps:?}");
        assert_eq!(LIBRARY_READ_CAP, 16 * 1024 * 1024);
    }

    // ADR-0400 decision 20: the built-in mount's name is matched ASCII case-insensitively on every OS, and what follows
    //   it — an id, the folder `settings` — exactly, by `list` (with and without a query) and `read` alike.
    #[test]
    fn the_builtin_mount_name_matches_in_any_case_and_what_follows_it_exactly() {
        let dir = crate::builtin::tests::full();
        let store = TestStore::new(&["/lib"]);
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), Some(dir.path()), io);
        let query = |text: &str| ListFilter {
            query: Some(text.to_owned()),
            ..ListFilter::default()
        };
        for top in ["engram", "Engram", "ENGRAM"] {
            let listed = svc.list(Some(&lp(top))).unwrap();
            assert_eq!(
                paths(&listed),
                ["engram/default", "engram/priming"],
                "{top}"
            );
            assert_eq!(folder_paths(&listed), ["engram/settings"], "{top}");
            let settings = svc.list(Some(&lp(&format!("{top}/settings")))).unwrap();
            assert_eq!(paths(&settings), ["engram/settings/default"], "{top}");
            let found = svc.list_with(Some(&lp(top)), &query("default")).unwrap();
            assert_eq!(
                paths(&found),
                ["engram/default", "engram/settings/default"],
                "{top}"
            );
            let below = svc
                .list_with(Some(&lp(&format!("{top}/settings"))), &query("default"))
                .unwrap();
            assert_eq!(paths(&below), ["engram/settings/default"], "{top}");
            let read = svc.read(&lp(&format!("{top}/settings/default"))).unwrap();
            assert_eq!(read.row.path, "engram/settings/default", "{top}");
        }
        // A query answers strictly below the folder asked, as in a library mount: a built-in's own path holds nothing.
        let own = svc
            .list_with(Some(&lp("engram/default")), &query("default"))
            .unwrap();
        assert!(own.rows.is_empty(), "{:?}", own.rows);
        for folder in ["engram/SETTINGS", "Engram/Settings"] {
            assert!(
                svc.list(Some(&lp(folder))).unwrap().rows.is_empty(),
                "{folder}"
            );
            let found = svc.list_with(Some(&lp(folder)), &query("default")).unwrap();
            assert!(found.rows.is_empty(), "{folder}");
        }
        for path in [
            "engram/SETTINGS/default",
            "engram/Default",
            "ENGRAM/Priming",
        ] {
            let (class, reason) = refused(svc.read(&lp(path)).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::NotFound, NOT_FOUND),
                "{path}"
            );
        }
    }

    // ADR-0400 decision 20: the built-in mount's row tells what its `read` does — with no built-in folder, or one none of
    //   whose files can be read (here missing), its `list` and `folders` row is unreadable, a whole-tree read names it
    //   in `unavailable` and `read engram/…` refuses naming it, while its items stay listed as stand-ins; with the
    //   folder it is `Ok` and named nowhere.
    #[test]
    fn the_builtin_mount_row_is_unreadable_exactly_when_its_read_refuses() {
        let full = crate::builtin::tests::full();
        let empty = tempfile::tempdir().unwrap();
        let missing = empty.path().join("builtin");
        let unreadable = MountState::Unreadable {
            why: crate::builtin::FOLDER_UNREADABLE.to_owned(),
        };
        for (dir, state) in [
            (None, no_builtin_folder()),
            (Some(missing.as_path()), unreadable),
            (Some(full.path()), MountState::Ok),
        ] {
            let store = TestStore::new(&["/lib"]);
            let io = BoundedIo::new(
                IO_LIMIT,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::new(StatOps::default()),
                Arc::new(ThreadSpawn),
            );
            let svc = service_io(Some("/lib"), &store, Vec::new(), dir, io);
            let readable = state == MountState::Ok;
            let unavailable = match readable {
                false => vec![UnavailableMount {
                    mount: BUILTIN_MOUNT.to_owned(),
                    state: state.clone(),
                }],
                true => Vec::new(),
            };
            let top = svc.list(None).unwrap();
            assert_eq!(top.folders[0].path, BUILTIN_MOUNT);
            assert_eq!(top.folders[0].mount_state, Some(state.clone()));
            let folders = svc.folders().unwrap();
            assert_eq!(folders.folders[0].path, BUILTIN_MOUNT);
            assert_eq!(folders.folders[0].mount_state, Some(state.clone()));
            assert_eq!(folders.unavailable, unavailable);
            let found = svc
                .list_with(
                    None,
                    &ListFilter {
                        query: Some("default".to_owned()),
                        ..ListFilter::default()
                    },
                )
                .unwrap();
            assert_eq!(paths(&found), ["engram/default", "engram/settings/default"]);
            assert_eq!(found.unavailable, unavailable);
            let rows = svc.list(Some(&lp("engram"))).unwrap().rows;
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows.iter().all(|row| row.state.is_some()),
                !readable,
                "stand-ins exactly when the built-in folder cannot be read: {rows:?}"
            );
            let read = svc.read(&lp("engram/priming"));
            match readable {
                false => {
                    let (class, reason) = refused(read.unwrap_err());
                    assert_eq!(
                        (class, reason.code.as_str(), &reason.args["mount"]),
                        (
                            RefusalClass::Unavailable,
                            LIBRARY_UNAVAILABLE,
                            &json!(BUILTIN_MOUNT)
                        )
                    );
                    assert!(
                        reason.args["message"]
                            .as_str()
                            .unwrap()
                            .ends_with(&state.why()),
                        "{reason:?}"
                    );
                }
                true => assert_eq!(read.unwrap().row.path, "engram/priming"),
            }
        }
    }

    // A built-in folder some of whose files read is readable: only the missing built-in stands in.
    #[test]
    fn a_builtin_folder_with_one_file_missing_is_readable() {
        let dir = crate::builtin::tests::full();
        std::fs::remove_file(dir.path().join(crate::builtin::file_of(BuiltinId::Priming))).unwrap();
        let store = TestStore::new(&["/lib"]);
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), Some(dir.path()), io);
        let top = svc.list(None).unwrap();
        assert_eq!(top.folders[0].mount_state, Some(MountState::Ok));
        let read = svc.read(&lp("engram/priming")).unwrap();
        assert!(read.row.state.is_some(), "the priming stands in");
    }

    // ADR-0400 decision 20: a built-in file that reads but does not parse is that item's state, not the mount's.
    #[test]
    fn a_builtin_file_that_reads_but_does_not_parse_keeps_the_builtin_mount_ok() {
        let dir = crate::builtin::tests::full();
        let preset = dir
            .path()
            .join(crate::builtin::file_of(BuiltinId::DefaultPreset));
        std::fs::write(&preset, "kind: preset\nversion: [\n").unwrap();
        let store = TestStore::new(&["/lib"]);
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), Some(dir.path()), io);
        let top = svc.list(None).unwrap();
        let engram = top
            .folders
            .iter()
            .find(|row| row.path == BUILTIN_MOUNT)
            .expect("the built-in mount row");
        assert_eq!(engram.mount_state, Some(MountState::Ok));
        assert_eq!(
            mount_row(&svc.status(), BUILTIN_MOUNT).state,
            MountState::Ok
        );
        let read = svc.read(&lp("engram/default")).unwrap();
        assert!(
            read.row.state.is_some(),
            "the preset that does not parse shows its state"
        );
        assert!(
            svc.folders().unwrap().unavailable.is_empty(),
            "a whole-tree read names no unusable mount"
        );
    }

    // ADR-0400 decision 20: the built-in mount is never scanned — a refresh of it walks nothing and says so.
    #[test]
    fn a_refresh_of_the_builtin_mount_walks_nothing_and_says_it_is_not_scanned() {
        let (svc, store) = library();
        svc.first_scan_join().unwrap();
        let walked = store.scans().len();
        for folder in ["engram", "Engram/settings"] {
            let report = svc.refresh(Some(&lp(folder)));
            let [(None, why)] = report.errors.as_slice() else {
                panic!("{report:?}")
            };
            assert!(
                why.contains("built-in mount engram is not scanned"),
                "{why}"
            );
            assert_eq!(report.changed, 0);
        }
        assert_eq!(store.scans().len(), walked, "nothing walked");
    }

    // ADR-0400 decision 4: with no mount registered the virtual root still reads — only the built-in mount, never
    //   `library-unavailable`.
    #[test]
    fn without_a_mount_a_refresh_and_a_list_answer_only_the_builtin_mount() {
        let (svc, store) = service_on(&["/lib"], None);
        assert_eq!(svc.refresh(None), RefreshReport::default());
        let top = svc.list(None).unwrap();
        assert!(top.rows.is_empty(), "{top:?}");
        assert_eq!(folder_paths(&top), ["engram"]);
        assert!(svc.list(Some(&lp("library"))).unwrap().rows.is_empty());
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
            svc.list(Some(&lp("library"))).unwrap();
            let read = check_read(&svc, "library/d/x.md");
            store.mem("/lib").remove("d/x.md");
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("library/d"))));
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
            assert!(!has(&svc, "library/d/x.md"), "merge first: {merge_first}");
            assert!(has(&svc, "library/d/y.md"));
        }
    }

    #[test]
    fn a_file_created_after_the_scan_listed_its_folder_lands_per_order_and_the_safety_net_inserts_it_later(
    ) {
        for merge_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file("d/y.md", "y");
            svc.list(Some(&lp("library"))).unwrap();
            store.hold_at(At::End, "/lib");
            let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("library/d"))));
            store.wait_held();
            store.mem("/lib").file("d/x.md", "x");
            let read = check_read(&svc, "library/d/x.md");
            if merge_first {
                merge(&svc, read);
                store.release();
                join(scan);
                assert!(has(&svc, "library/d/x.md"), "read after the listing: kept");
            } else {
                store.release();
                join(scan);
                merge(&svc, read);
                assert!(
                    !has(&svc, "library/d/x.md"),
                    "a swap-in landed between: the insert is dropped"
                );
                svc.list(Some(&lp("library/d/x.md"))).unwrap();
                assert!(
                    has(&svc, "library/d/x.md"),
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
        svc.list(Some(&lp("library"))).unwrap();
        store.mem("/lib").file_version("b/y.md", "y2", 2);
        let read = check_read(&svc, "library/b/y.md");
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(Some(&lp("library/a"))));
        store.wait_held();
        merge(&svc, read);
        store.release();
        join(scan);
        assert_eq!(version(&svc, "library/b/y.md"), 2);
    }

    #[test]
    fn a_merge_inside_the_range_read_later_survives_the_whole_scan_apply() {
        let (svc, store) = library();
        store.mem("/lib").file_version("x.md", "v1", 1);
        svc.list(Some(&lp("library"))).unwrap();
        // The scan lists x at version 2 and reads it only after the hold — by then it holds version 3.
        store.mem("/lib").file_version("x.md", "v2", 2);
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file_version("x.md", "v3", 3);
        merge(&svc, check_read(&svc, "library/x.md"));
        store.release();
        join(scan);
        assert_eq!(version(&svc, "library/x.md"), 3, "the later read stays");
    }

    #[test]
    fn a_safety_net_insert_landing_during_a_walk_survives_its_apply() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        svc.list(Some(&lp("library"))).unwrap();
        store.hold_at(At::End, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        store.mem("/lib").file("n.md", "n");
        svc.list(Some(&lp("library/n.md"))).unwrap();
        assert!(has(&svc, "library/n.md"));
        store.release();
        join(scan);
        assert!(has(&svc, "library/n.md"), "read after the listing");
    }

    #[test]
    fn with_the_numbers_unchanged_the_later_read_wins_in_either_landing_order() {
        for later_first in [false, true] {
            let (svc, store) = library();
            store.mem("/lib").file_version("x.md", "v1", 1);
            svc.list(Some(&lp("library"))).unwrap();
            store.mem("/lib").file_version("x.md", "v2", 2);
            let older = check_read(&svc, "library/x.md");
            store.mem("/lib").file_version("x.md", "v3", 3);
            let newer = check_read(&svc, "library/x.md");
            if later_first {
                merge(&svc, newer);
                merge(&svc, older);
            } else {
                merge(&svc, older);
                merge(&svc, newer);
            }
            assert_eq!(
                version(&svc, "library/x.md"),
                3,
                "later first: {later_first}"
            );
        }
    }

    #[test]
    fn a_merge_read_before_a_change_does_not_revive_what_the_change_removed() {
        let (svc, store) = library();
        store.mem("/lib").file("p.md", "p");
        svc.list(Some(&lp("library"))).unwrap();
        let read = check_read(&svc, "library/p.md");
        svc.change(|catalog, _, seq| {
            store.mem("/lib").remove("p.md");
            let k = key(&lp("library/p.md"));
            let removed = catalog.remove(&k, seq());
            ((), if removed { vec![k] } else { Vec::new() })
        });
        merge(&svc, read);
        assert!(!has(&svc, "library/p.md"));
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
        svc.list(Some(&lp("library"))).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "library/x.md");
        {
            let held = svc.gate.lib().write().unwrap();
            // The hand-off returns while the lock is held (merging in place, it would hang here).
            join(on_thread(&svc, move |svc| svc.hand_off_merge(noted, items)));
            let still = held.catalog.get(&key(&lp("library/x.md"))).unwrap().tag;
            assert_eq!(
                still.modified,
                OLD + Duration::from_secs(1),
                "not merged yet"
            );
        }
        spawn.join_all();
        assert_eq!(version(&svc, "library/x.md"), 2, "delayed, not dropped");
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
        svc.list(Some(&lp("library"))).unwrap();
        store.mem("/lib").file_version("x.md", "v2", 2);
        let (noted, items) = check_read(&svc, "library/x.md");
        svc.hand_off_merge(noted, items);
        assert_eq!(version(&svc, "library/x.md"), 2);
    }

    // ── changes ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_change_drawn_during_a_walk_waits_for_its_apply_and_is_not_undone() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(Some(&lp("library"))).unwrap();
        store.hold_at(At::Start, "/lib");
        let scan = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        let disk = Arc::clone(&store);
        let change = on_thread(&svc, move |svc| {
            svc.change(|catalog, _, seq| {
                disk.mem("/lib").remove("x.md");
                let k = key(&lp("library/x.md"));
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
        assert!(!has(&svc, "library/x.md"));
    }

    #[test]
    fn a_change_generation_moved_during_a_walk_makes_the_scan_walk_again() {
        let (svc, store) = library();
        store.mem("/lib").file("x.md", "x");
        svc.list(Some(&lp("library"))).unwrap();
        store.hold_at(At::Start, "/lib");
        let walking = on_thread(&svc, |svc| svc.refresh(None));
        store.wait_held();
        svc.gate.change_unordered(|catalog| {
            store.mem("/lib").remove("x.md");
            catalog.remove(&key(&lp("library/x.md")), svc.gate.next_seq());
        });
        store.release();
        join(walking);
        assert!(!has(&svc, "library/x.md"));
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/lib", None), scan("/lib", None)]
        );
    }

    // ── mount change ─────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn readers_see_the_old_folder_during_a_relocated_mounts_walk() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        store.mem("/lib2").file("b.md", "b");
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/a.md"]
        );
        store.hold_at(At::Start, "/lib2");
        let change = on_thread(&svc, |svc| relocate(svc, "library", "/lib2"));
        store.wait_held();
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/a.md"],
            "never an empty catalog"
        );
        store.release();
        join(change).unwrap();
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/b.md"]
        );
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib2")));
    }

    #[test]
    fn a_nested_relocation_keeps_a_file_whose_tag_equals_an_old_item_of_the_same_key() {
        let (svc, store) = service_on(&["/lib", "/lib/team"], Some("/lib"));
        // `X.md` under the old root and `team/X.md` (the new root's `X.md`) have equal tags: same size, same time.
        store.mem("/lib").file_version("X.md", "top!", 1);
        store.mem("/lib").file_version("team/X.md", "team", 1);
        store.mem("/lib/team").file_version("X.md", "team", 1);
        assert_eq!(
            svc.list(Some(&lp("library"))).unwrap().rows[0].description,
            excerpt("top!")
        );
        relocate(&svc, "library", "/lib/team").unwrap();
        let rows = svc.list(Some(&lp("library"))).unwrap();
        assert_eq!(paths(&rows), ["library/X.md"]);
        assert_eq!(rows.rows[0].description, excerpt("team"));
    }

    // ADR-0400 decision 16: a mount change walks one mount, not the library — the first scan stays a whole scan.
    #[test]
    fn a_mount_added_before_the_first_scan_leaves_the_first_scan_to_a_whole_walk() {
        let (svc, store) = service_on(&["/lib"], None);
        store.mem("/lib").file("a.md", "a");
        let mount = Mount {
            name: MountName::library(),
            folder: PathBuf::from("/lib"),
        };
        svc.gate
            .change_mount(MountChange::Add(mount), |mount| {
                svc.build_scan(mount, None, &Tags::new())
            })
            .unwrap();
        assert!(
            has(&svc, "library/a.md"),
            "the added mount was walked and applied"
        );
        assert_eq!(svc.numbers().swap_ins, 0, "no scan was applied");
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/a.md"]
        );
        assert_eq!(
            store.scans(),
            [scan("/lib", None), scan("/lib", None)],
            "the reader's first scan walked the library"
        );
    }

    #[test]
    fn a_mount_relocation_whose_walk_fails_at_the_folder_publishes_nothing() {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        store.mem("/lib").file("a.md", "a");
        svc.list(Some(&lp("library"))).unwrap();
        store.mem("/lib2").fail_listing("", ListFailure::Open);
        let before = svc.numbers();
        assert!(relocate(&svc, "library", "/lib2").is_err());
        assert_eq!(svc.numbers(), before, "no number moved");
        let answer = join(on_thread(&svc, |svc| {
            svc.list(Some(&lp("library"))).unwrap()
        }));
        assert_eq!(paths(&answer), ["library/a.md"]);
        assert_eq!(root_of(&svc), Some(PathBuf::from("/lib")));
        let report = join(on_thread(&svc, |svc| svc.refresh(None)));
        assert!(report.errors.is_empty(), "the ticket was released");
    }

    // ── resolution entrances ─────────────────────────────────────────────────────────────────────────────

    /// Folder ops that count every stat by path, answer from a table (default `Ok`), and hold the stat of a held path
    /// until the test releases it.
    #[derive(Default)]
    struct CountOps {
        statuses: HashMap<PathBuf, FolderStatus>,
        /// Statuses set after the setup; they win over `statuses`.
        later: Mutex<HashMap<PathBuf, FolderStatus>>,
        /// From the stat number `n` (counted from 1) of `path` on, `path` answers this status; it wins over the others.
        from: Mutex<Option<(PathBuf, usize, FolderStatus)>>,
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
            let number = {
                let mut stats = lock(&self.stats);
                let count = stats.entry(p.to_path_buf()).or_default();
                *count += 1;
                *count
            };
            if let Some((_, _, status)) = lock(&self.from)
                .as_ref()
                .filter(|(path, n, _)| path == p && number >= *n)
            {
                return status.clone();
            }
            if self.held.iter().any(|held| held == p) {
                let deadline = Instant::now() + Duration::from_secs(20);
                let mut released = lock(&self.released);
                while !*released && Instant::now() < deadline {
                    released = wait_timeout(&self.moved, released, deadline - Instant::now()).0;
                }
            }
            let later = lock(&self.later).get(p).cloned();
            later
                .or_else(|| self.statuses.get(p).cloned())
                .unwrap_or(FolderStatus::Ok)
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, _: &Path) -> io::Result<PathBuf> {
            Err(io::Error::other("not modeled"))
        }

        fn identity(&self, _: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
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

    /// The built-in mount's state in a service with no built-in folder.
    fn no_builtin_folder() -> MountState {
        MountState::Unreadable {
            why: crate::builtin::NO_FOLDER.to_owned(),
        }
    }

    /// The ④ row of `mount`.
    fn library_unavailable(mount: &str, message: &str) -> BlockReason {
        BlockReason::LibraryUnavailable {
            reason: Reason {
                code: LIBRARY_UNAVAILABLE.to_owned(),
                args: json!({ "mount": mount, "message": message }),
                line: None,
            },
        }
    }

    fn absent(mount: &str, why: &str) -> Vec<(String, String)> {
        vec![(mount.to_owned(), why.to_owned())]
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
                "kind: preset\nversion: 1\ninstructions:\n  - library/a.md\nfolders:\n  - '{spelled}'\n"
            ),
        );
        mem.file("a.md", "a");
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            store.scans(),
            [scan("/lib", None)],
            "the first scan, joined"
        );
        assert_eq!(
            resolution.instructions,
            [("library/a.md".to_owned(), Path::new("/lib").join("a.md"))]
        );
        assert_eq!(resolution.folders[0].status, Some(FolderStatus::Ok));
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert_eq!(
            (ops.stats_of(Path::new("/lib")), ops.stats_of(&folder)),
            (2, 1),
            "the root: the first scan's stat before its ticket, and the resolution's"
        );
        svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(store.scans(), [scan("/lib", None)], "no second scan");
    }

    #[test]
    fn resolve_agent_answers_an_agent_with_a_settings_check_and_reads_the_builtins_once() {
        let dir = crate::builtin::tests::full();
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/a.md\n",
        );
        store.mem("/lib").file("a.md", "a");
        let claude = claude_json(&["library/P.yaml"]);
        let shell = agent(&["library/P.yaml"]);
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
        assert_eq!(resolution.instructions[0].0, "library/a.md");
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
            claude_json(&["library/P.yaml"]),
            claude_json(&["library/Q.yaml"]),
            claude_json(&["library/Q.yaml", "library/P.yaml"]),
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
                let BlockReason::Broken { reason, .. } = problem else {
                    panic!("{problem:?}")
                };
                assert_eq!(reason.code, resolve::FOLDER_UNREADABLE);
                assert_eq!(reason.args["why"], folder::TIMEOUT);
            }
        }
        assert_eq!(
            codes(all[2].1.as_ref().unwrap()).len(),
            2,
            "each folder once"
        );
    }

    /// `agent.setWorkspace`'s folder stat goes through the bounded IO on a fresh read budget: each status as its word, and
    /// a stat that does not answer within the budget is `timeout` — the call returns at the budget, not when the stat
    /// ends.
    // ADR-0360
    // ADR-0386
    #[test]
    fn the_folder_stat_is_bounded_by_one_read_budget_and_names_a_timeout() {
        let store = TestStore::new(&["/lib"]);
        let held = PathBuf::from("/slow");
        let ops = Arc::new(CountOps {
            statuses: HashMap::from([
                (PathBuf::from("/gone"), FolderStatus::Missing),
                (PathBuf::from("/file"), FolderStatus::NotADirectory),
                (
                    PathBuf::from("/locked"),
                    FolderStatus::Unreadable("access denied".to_owned()),
                ),
            ]),
            held: vec![held.clone()],
            ..CountOps::default()
        });
        let limit = Duration::from_millis(300);
        let io = BoundedIo::new(
            limit,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), None, io);

        assert_eq!(svc.stat_folder(Path::new("/work")), Ok(()));
        assert_eq!(
            svc.stat_folder(Path::new("/gone")),
            Err("missing".to_owned())
        );
        assert_eq!(
            svc.stat_folder(Path::new("/file")),
            Err("not a folder".to_owned())
        );
        assert_eq!(
            svc.stat_folder(Path::new("/locked")),
            Err("access denied".to_owned())
        );
        let started = Instant::now();
        let slow = svc.stat_folder(&held);
        let spent = started.elapsed();
        ops.release();
        assert_eq!(slow, Err(folder::TIMEOUT.to_owned()));
        assert!(spent < limit + Duration::from_secs(1), "{spent:?}");
        assert_eq!(ops.stats_of(&held), 1);
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
            statuses: HashMap::from([(folder.clone(), FolderStatus::Missing)]),
            ..CountOps::default()
        });
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let library = claude_json(&["library/P.yaml"]);
        let builtin_only = claude_json(&[]);
        let svc = service_io(
            Some("/lib"),
            &store,
            vec![library.clone(), builtin_only.clone()],
            None,
            io,
        );
        svc.first_scan_join().unwrap();
        assert!(has(&svc, "library/P.yaml"), "a good first scan");
        assert_eq!(
            ops.stats_of(Path::new("/lib")),
            1,
            "the first scan's stat before its ticket"
        );
        lock(&ops.later).insert(PathBuf::from("/lib"), FolderStatus::Missing);
        assert!(svc
            .resolve_agent(builtin_only.id)
            .unwrap()
            .problems
            .is_empty());
        assert_eq!(
            ops.stats_of(Path::new("/lib")),
            1,
            "no library preset — not asked"
        );
        let absent = svc.resolve_agent(library.id).unwrap();
        // ADR-0400 decision 19: block ④ stands for the mount's items only — the built-in default still resolves (its
        //   stand-in notice here: no built-in folder).
        let notices: Vec<&str> = absent.notices.iter().map(|n| n.code.as_str()).collect();
        assert_eq!(notices, [BUILTIN_UNAVAILABLE]);
        assert_eq!(
            absent,
            Resolution {
                problems: vec![library_unavailable("library", "missing")],
                notices: absent.notices.clone(),
                absent_mounts: self::absent("library", "missing"),
                ..Resolution::default()
            },
            "one block ④ row for the mount — no folder, no folder row"
        );
        assert_eq!(
            absent.skipped(),
            absent.problems,
            "an absent mount is skipped (ADR-0387)"
        );
        assert_eq!(
            ops.stats_of(Path::new("/lib")),
            2,
            "stat'ed once more, and not walked again — absent"
        );
        assert_eq!(
            ops.stats_of(&folder),
            0,
            "block ④'s folders are not stat'ed"
        );

        // No mount at all: the preset's first name is not registered — absent, block ④ alone, skipped.
        let (unset, _) = service_on(&["/lib"], None);
        let resolution = unset.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            resolution.problems,
            [library_unavailable("library", resolve::NOT_REGISTERED)]
        );
        assert_eq!(
            resolution.absent_mounts,
            self::absent("library", resolve::NOT_REGISTERED)
        );
        assert!(resolution.blocked().is_none());

        // ADR-0387: a mount there whose walk keeps failing is unreadable: its reason alone, blocked; the preset under a
        //   link gets no path-through-link row. Stat'ed by the first scan, by the reader, and twice by the reader's
        //   walk again — its fresh stat before any ticket, then its scan's (ADR-0400 decision 17).
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
        *lock(&store.fail_every_scan) = true;
        let resolution = svc.resolve_preset(&lp("library/lnk/P.yaml"));
        let [BlockReason::LibraryUnavailable { reason }] = resolution.problems.as_slice() else {
            panic!("{:?}", resolution.problems)
        };
        let message = reason.args["message"].as_str().unwrap();
        assert!(
            !message.is_empty() && message != resolve::NOT_REGISTERED && message != "missing",
            "{message}"
        );
        assert_eq!(reason.args["mount"], "library");
        assert!(resolution.absent_mounts.is_empty() && resolution.blocked().is_some());
        assert_eq!(ops.stats_of(Path::new("/lib")), 4);

        // A first scan that failed on a root that is not there: absent, skipped.
        let store = TestStore::new(&["/lib"]);
        let ops = Arc::new(CountOps {
            statuses: HashMap::from([(PathBuf::from("/lib"), FolderStatus::Missing)]),
            ..CountOps::default()
        });
        let io = BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let svc = service_io(Some("/lib"), &store, Vec::new(), None, io);
        *lock(&store.fail_next_scan) = true;
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            resolution.problems,
            [library_unavailable("library", "missing")]
        );
        assert_eq!(resolution.skipped(), resolution.problems);
        assert_eq!(
            ops.stats_of(Path::new("/lib")),
            2,
            "the first scan's stat and the reader's; absent, so not walked"
        );
        assert!(
            store.scans().is_empty(),
            "a mount that is not there is not walked"
        );
    }

    /// A link probe that fails turns a `not-found` row into ② `library-read-failed` — the path could not be checked,
    /// so it is not known to be absent; a path under a file (`NotADirectory`) stays `not-found` (ADR-0387).
    #[test]
    fn a_failed_link_probe_is_a_read_failure_except_under_a_file() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - library/gone.md\n",
        );
        *lock(&store.fail_link) = Some(io::ErrorKind::PermissionDenied);
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        let [BlockReason::Broken { item, via, reason }] = resolution.problems.as_slice() else {
            panic!("{:?}", resolution.problems)
        };
        assert_eq!(
            (item.as_str(), via.as_slice()),
            ("library/gone.md", ["library/P.yaml".to_owned()].as_slice())
        );
        assert_eq!(reason.code, launch::LIBRARY_READ_FAILED);
        assert_eq!(reason.args["path"], "library/gone.md");
        assert!(reason.args["why"]
            .as_str()
            .is_some_and(|why| !why.is_empty()));
        assert_eq!(reason.line, Some(4));

        *lock(&store.fail_link) = Some(io::ErrorKind::NotADirectory);
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(codes(&resolution), [NOT_FOUND]);
        assert_eq!(resolution.skipped(), resolution.problems);
    }

    #[test]
    fn a_settings_rows_state_shows_a_refusal_in_any_backends_field() {
        let (svc, store) = library();
        store.mem("/lib").file(
            "s.yaml",
            "kind: settings\nversion: 1\nclaude: {}\ncodex:\n  - foo\n",
        );
        let rows = svc.list(Some(&lp("library"))).unwrap().rows;
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
            "kind: preset\nversion: 1\ninstructions:\n  - library/lnk/x.md\n  - library/gone.md\n",
        );
        let resolution = svc.resolve_preset(&lp("library/P.yaml"));
        assert_eq!(
            resolution.problems,
            [
                BlockReason::Broken {
                    item: "library/lnk/x.md".to_owned(),
                    via: vec!["library/P.yaml".to_owned()],
                    reason: Reason {
                        code: PATH_THROUGH_LINK.to_owned(),
                        args: json!({ "path": "library/lnk/x.md" }),
                        line: Some(4),
                    },
                },
                BlockReason::Missing {
                    item: "library/gone.md".to_owned(),
                    via: vec!["library/P.yaml".to_owned()],
                    reason: Reason {
                        code: NOT_FOUND.to_owned(),
                        args: json!({ "path": "library/gone.md" }),
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
        store.mem("/lib").file(
            "P.yaml",
            "kind: preset\nversion: 1\nskills:\n  - library/s\n",
        );
        store
            .mem("/lib")
            .file("s/SKILL.md", "---\nname: s\ndescription: \"\"\n---\n");
        let codex = |mode| {
            coded(
                AgentCommand::Codex {
                    extra_args: Vec::new(),
                    output_format: mode,
                },
                &["library/P.yaml"],
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

    // ── the mount setting: status · the mount commands · first start ─────────────────────────────────────

    /// A service whose `library.json` lives in a real temp folder (`<temp>/state`). The test store's roots are
    /// absolute paths under the temp folder (named by [`OnDisk::at`]); nothing of them is on disk unless a test makes
    /// it, and their stats answer from `ops` (default `Ok`; canonical paths are the spellings).
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

        /// A `library.json` holding `mounts` (name, folder name — `default` = the word), in order.
        fn mounts_doc(&self, mounts: &[(&str, &str)]) -> String {
            let entries: Vec<serde_json::Value> = mounts
                .iter()
                .map(|(name, folder)| match *folder {
                    "default" => json!({ "name": name, "path": "default" }),
                    folder => json!({ "name": name, "path": self.at(folder).to_str().unwrap() }),
                })
                .collect();
            json!({ "schema_version": 2, "mounts": entries }).to_string()
        }

        fn explicit(&self, name: &str) -> String {
            self.mounts_doc(&[("library", name)])
        }

        /// A new service over a fresh `config::load` of the state folder — one daemon start, at [`ROOT_BASE`].
        fn start(&self) -> Arc<LibraryService> {
            self.start_with(ROOT_BASE, Vec::new())
        }

        /// [`OnDisk::start`] at the root generation base `base`.
        fn start_at(&self, base: u64) -> Arc<LibraryService> {
            self.start_with(base, Vec::new())
        }

        /// [`OnDisk::start`] at `base`, its registry holding `profiles`.
        fn start_with(&self, base: u64, profiles: Vec<AgentProfile>) -> Arc<LibraryService> {
            self.start_on(base, profiles, IO_LIMIT)
        }

        /// [`OnDisk::start`] whose read budget is `limit`.
        fn start_limited(&self, limit: Duration) -> Arc<LibraryService> {
            self.start_on(ROOT_BASE, Vec::new(), limit)
        }

        fn start_on(
            &self,
            base: u64,
            profiles: Vec<AgentProfile>,
            limit: Duration,
        ) -> Arc<LibraryService> {
            let registry = Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles))));
            self.start_over(base, registry, limit)
        }

        /// [`OnDisk::start`] over the registry `registry`.
        fn start_registry(&self, registry: Arc<ProfileRegistry>) -> Arc<LibraryService> {
            self.start_over(ROOT_BASE, registry, IO_LIMIT)
        }

        fn start_over(
            &self,
            base: u64,
            registry: Arc<ProfileRegistry>,
            limit: Duration,
        ) -> Arc<LibraryService> {
            let io = BoundedIo::new(
                limit,
                LaunchWaits::LAUNCH,
                LIST_CAP,
                Arc::clone(&self.ops) as Arc<dyn FolderOps>,
                Arc::new(ThreadSpawn),
            );
            let values = LibraryValues {
                registry,
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

    /// The first stored mount's `path`.
    fn word(disk: &OnDisk) -> serde_json::Value {
        disk.config().expect("library.json")["mounts"][0]["path"].clone()
    }

    /// The stored mounts as (name, path word).
    fn stored(disk: &OnDisk) -> Vec<(String, String)> {
        let doc = disk.config().expect("library.json");
        doc["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mount| {
                (
                    mount["name"].as_str().unwrap().to_owned(),
                    mount["path"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The status row of the mount `name`.
    fn mount_row<'s>(status: &'s LibraryStatus, name: &str) -> &'s MountRow {
        status
            .mounts
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("no row {name}: {status:?}"))
    }

    /// The names of the status rows, in order.
    fn row_names(status: &LibraryStatus) -> Vec<&str> {
        status.mounts.iter().map(|row| row.name.as_str()).collect()
    }

    /// The built-in row of a test service (no built-in folder).
    fn builtin_row() -> MountRow {
        MountRow {
            name: BUILTIN_MOUNT.to_owned(),
            folder: PathBuf::new(),
            default: false,
            builtin: true,
            state: no_builtin_folder(),
        }
    }

    /// `svc.status()` asked until the row of `name` is no longer in `from` — a stat worker of an earlier status may
    /// still be in flight, and a required ask joins it (`folder.rs` — one worker per key).
    fn settled(svc: &LibraryService, name: &str, from: &MountState) -> MountState {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = mount_row(&svc.status(), name).state.clone();
            if state != *from || Instant::now() > deadline {
                break state;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    // ADR-0400 decision 7
    #[test]
    fn a_status_answers_every_mount_in_order_then_the_builtin_with_its_place_and_a_fresh_state() {
        let disk = OnDisk::new(&["library", "lib"], "library", None);
        let svc = disk.start();
        let base = svc.numbers().root;
        assert_eq!(base, ROOT_BASE, "the run starts at the base it was given");
        assert_eq!(
            svc.status(),
            LibraryStatus {
                daemon_id: "daemon-1".to_owned(),
                mounts: vec![builtin_row()],
                config: ConfigHealth::Ok,
                suggested_new: disk.default_root.clone(),
                root_generation: base,
            },
            "before the first start only the built-in is a mount"
        );
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("team", "lib"), ("library", "default")]),
        )
        .unwrap();
        let svc = disk.start();
        lock(&disk.ops.0).insert(disk.at("lib"), FolderStatus::NotADirectory);
        let status = svc.status();
        assert_eq!(
            status.mounts,
            [
                MountRow {
                    name: "team".to_owned(),
                    folder: disk.at("lib"),
                    default: false,
                    builtin: false,
                    state: MountState::Missing {
                        why: "not a folder".to_owned()
                    },
                },
                MountRow {
                    name: "library".to_owned(),
                    folder: disk.at("library"),
                    default: true,
                    builtin: false,
                    state: MountState::Ok,
                },
                builtin_row(),
            ],
            "list order, the default place told, each folder stat'ed now"
        );
        lock(&disk.ops.0).insert(
            disk.at("lib"),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        let missing = MountState::Missing {
            why: "not a folder".to_owned(),
        };
        assert_eq!(
            settled(&svc, "team", &missing),
            MountState::Unreadable {
                why: "denied".to_owned()
            },
            "a mount that exists but cannot be read is unreadable, with why (ADR-0387)"
        );
    }

    // ADR-0400 decision 9
    #[test]
    fn add_mount_registers_a_folder_walks_it_saves_every_mount_and_counts_what_already_pointed_at_it(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "default")]),
        )
        .unwrap();
        disk.mem("library").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/x.md\n  - team/gone.md\n  - library/a.md\n",
        );
        disk.mem("library").file("a.md", "a");
        disk.mem("lib2").file("x.md", "x");
        disk.mem("lib2").file(
            "Q.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/x.md\n",
        );
        let pointing = agent(&["team/Q.yaml"]);
        let elsewhere = agent(&["library/P.yaml"]);
        let svc = disk.start_with(ROOT_BASE, vec![pointing, elsewhere]);
        svc.first_scan_join().unwrap();
        let changes = svc.numbers().change;

        let added = svc.add_mount("team", &disk.at("lib2"), false).unwrap();

        assert_eq!(
            added.pointing,
            Pointing {
                presets: 2,
                agents: 1
            },
            "the references already naming `team` outside it — `team`'s own preset does not count"
        );
        assert_eq!(row_names(&added.status), ["library", "team", "engram"]);
        assert_eq!(mount_row(&added.status, "team").state, MountState::Ok);
        assert_eq!(added.status.root_generation, ROOT_BASE + 1);
        assert_eq!(
            svc.numbers().change,
            changes + 1,
            "the change number moves too"
        );
        assert_eq!(
            paths(&svc.list(Some(&lp("team"))).unwrap()),
            ["team/Q.yaml", "team/x.md"],
            "walked under its name"
        );
        assert!(has(&svc, "library/P.yaml"), "the other mount's items stay");
        assert_eq!(
            stored(&disk),
            [
                ("library".to_owned(), "default".to_owned()),
                ("team".to_owned(), disk.at("lib2").display().to_string())
            ]
        );
        assert_eq!(
            row_names(&disk.start().status()),
            ["library", "team", "engram"],
            "the next start reads it"
        );
    }

    // ADR-0400 decisions 2 and 7
    #[test]
    fn add_mount_refuses_an_invalid_name_a_relative_path_and_a_taken_name_with_nothing_done() {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        let doc = disk.mounts_doc(&[("library", "default")]);
        std::fs::write(disk.config_path(), &doc).unwrap();
        let svc = disk.start();
        for name in ["", "a/b", ".hidden", "engram", "Engram", "CON"] {
            let (class, reason) =
                refused(svc.add_mount(name, &disk.at("lib2"), false).unwrap_err());
            assert_eq!(
                (class, reason.code.as_str(), &reason.args["name"]),
                (
                    RefusalClass::InvalidArgument,
                    MOUNT_NAME_INVALID,
                    &json!(name)
                ),
                "{name:?}"
            );
        }
        let (class, reason) = refused(
            svc.add_mount("team", Path::new("relative/lib"), true)
                .unwrap_err(),
        );
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::InvalidArgument, ITEM_PATH_INVALID)
        );
        assert!(!Path::new("relative").exists());
        let taken = if same_path(Path::new("a"), Path::new("A")) {
            "LIBRARY"
        } else {
            "library"
        };
        let (class, reason) = refused(svc.add_mount(taken, &disk.at("lib2"), false).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str(), reason.args),
            (
                RefusalClass::Conflict,
                MOUNT_NAME_TAKEN,
                json!({ "name": "library" })
            ),
            "a name taken by path key, at another folder"
        );
        assert_eq!(std::fs::read_to_string(disk.config_path()).unwrap(), doc);
        assert_eq!(svc.numbers().root, ROOT_BASE, "no mount change ran");
    }

    // ADR-0400 decision 10 (as amended): only mounts are judged; a refused `create` leaves nothing on disk.
    #[test]
    fn add_and_relocate_refuse_a_folder_overlapping_a_mount_only_create_nothing_when_refused_and_relocate_skips_itself(
    ) {
        let disk = OnDisk::new(
            &["lib", "lib/sub", "lib2", "lib2/sub", "data/x"],
            "library",
            None,
        );
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "lib"), ("team", "lib2")]),
        )
        .unwrap();
        let svc = disk.start();
        let overlaps = |refusal: LibraryRefusal| {
            let (class, reason) = refused(refusal);
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Conflict, MOUNT_OVERLAPS)
            );
            reason.args
        };
        assert_eq!(
            overlaps(
                svc.add_mount("inner", &disk.at("lib/sub"), false)
                    .unwrap_err()
            ),
            json!({ "path": disk.at("lib/sub").display().to_string(), "with": "mount", "mount": "library" }),
            "inside a mount"
        );
        assert_eq!(
            overlaps(svc.add_mount("outer", disk.temp.path(), false).unwrap_err())["mount"],
            json!("library"),
            "holding the mounts (and the data folder): judged by the mounts alone"
        );
        let fresh = disk.at("lib/sub/fresh");
        assert_eq!(
            overlaps(svc.add_mount("fresh", &fresh, true).unwrap_err())["mount"],
            json!("library"),
            "a folder to be created is judged before it is made"
        );
        assert!(!fresh.exists(), "a refused add created nothing");
        let moved_in = disk.at("lib/moved");
        assert_eq!(
            overlaps(svc.relocate_mount("team", &moved_in, true).unwrap_err())["mount"],
            json!("library")
        );
        assert!(!moved_in.exists(), "a refused relocate created nothing");
        svc.add_mount("in-data", &disk.at("data/x"), false)
            .expect("a folder inside the data folder is a mount like any other");
        assert_eq!(
            overlaps(
                svc.relocate_mount("team", &disk.at("lib/sub"), false)
                    .unwrap_err()
            )["mount"],
            json!("library"),
            "a relocate is judged against every other mount"
        );
        let moved = svc
            .relocate_mount("team", &disk.at("lib2/sub"), false)
            .unwrap();
        assert_eq!(
            mount_row(&moved, "team").folder,
            disk.at("lib2/sub"),
            "a move into its own subfolder does not overlap itself"
        );
        assert_eq!(
            svc.numbers().root,
            ROOT_BASE + 2,
            "only the `in-data` add and the relocate ran"
        );
    }

    // ADR-0400 decision 10: a folder whose overlap cannot be judged is refused, never taken as free.
    #[test]
    fn a_folder_whose_canonical_path_cannot_be_had_is_refused_as_unusable_with_nothing_published() {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "default")]),
        )
        .unwrap();
        let svc = disk.start();
        lock(&disk.ops.1).push(disk.at("lib2"));
        let (class, reason) = refused(svc.add_mount("team", &disk.at("lib2"), false).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::Unavailable, LIBRARY_ROOT_UNUSABLE)
        );
        let why = reason.args["why"].as_str().unwrap();
        assert!(why.contains("canonical path"), "{why}");
        assert_eq!(row_names(&svc.status()), ["library", "engram"]);
        assert_eq!(svc.numbers().root, ROOT_BASE);
    }

    #[test]
    fn add_with_create_makes_the_folder_and_a_failed_create_stat_or_walk_publishes_and_writes_nothing(
    ) {
        let disk = OnDisk::new(
            &["library", "a/b/c", "blocker/lib", "gone", "broken"],
            "library",
            None,
        );
        let doc = disk.mounts_doc(&[("library", "default")]);
        std::fs::write(disk.config_path(), &doc).unwrap();
        std::fs::write(disk.temp.path().join("blocker"), "a file").unwrap();
        let svc = disk.start();
        svc.first_scan_join().unwrap();
        let unusable = |refusal: LibraryRefusal, path: PathBuf| {
            let (class, reason) = refused(refusal);
            assert_eq!(
                (class, reason.code.as_str(), &reason.args["path"]),
                (
                    RefusalClass::Unavailable,
                    LIBRARY_ROOT_UNUSABLE,
                    &json!(path.display().to_string())
                )
            );
            reason.args["why"].as_str().unwrap().to_owned()
        };
        unusable(
            svc.add_mount("x", &disk.at("blocker/lib"), true)
                .unwrap_err(),
            disk.at("blocker/lib"),
        );
        lock(&disk.ops.0).insert(disk.at("gone"), FolderStatus::Missing);
        let why = unusable(
            svc.add_mount("x", &disk.at("gone"), false).unwrap_err(),
            disk.at("gone"),
        );
        assert!(why.contains("missing"), "the stat's words: {why}");
        disk.mem("broken").fail_listing("", ListFailure::Open);
        unusable(
            svc.add_mount("x", &disk.at("broken"), false).unwrap_err(),
            disk.at("broken"),
        );
        assert_eq!(
            std::fs::read_to_string(disk.config_path()).unwrap(),
            doc,
            "nothing written"
        );
        assert_eq!(
            row_names(&svc.status()),
            ["library", "engram"],
            "nothing published"
        );
        assert_eq!(svc.numbers().root, ROOT_BASE, "no mount change ran");

        svc.add_mount("deep", &disk.at("a/b/c"), true).unwrap();
        assert!(disk.at("a/b/c").is_dir());
        assert_eq!(mount_row(&svc.status(), "deep").folder, disk.at("a/b/c"));
    }

    // ADR-0400 decision 15
    #[test]
    fn every_mount_command_on_a_read_only_config_refuses_before_anything() {
        let newer = r#"{"schema_version": 3, "mounts": []}"#;
        for unreadable in [false, true] {
            let disk = OnDisk::new(&["lib", "new"], "library", None);
            match unreadable {
                false => std::fs::write(disk.config_path(), newer).unwrap(),
                true => std::fs::create_dir(disk.config_path()).unwrap(),
            }
            let svc = disk.start();
            let refusals = [
                svc.add_mount("team", &disk.at("new"), true).map(drop),
                svc.remove_mount("team").map(drop),
                svc.relocate_mount("team", &disk.at("new"), true).map(drop),
            ];
            for refusal in refusals {
                let (class, reason) = refused(refusal.unwrap_err());
                assert_eq!(
                    (class, reason.code.as_str()),
                    (RefusalClass::Unavailable, LIBRARY_CONFIG_READ_ONLY),
                    "unreadable: {unreadable}"
                );
                assert!(
                    unreadable || reason.args["why"].as_str().unwrap().contains("newer"),
                    "{reason:?}"
                );
            }
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
            assert_eq!(status.mounts, [builtin_row()]);
            assert_eq!(svc.first_start(), FirstStart::NotDue);
        }
    }

    // ADR-0400 decision 11
    #[test]
    fn remove_mount_drops_the_registration_and_its_range_touches_no_file_and_adding_it_back_restores_it(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "default"), ("team", "lib2")]),
        )
        .unwrap();
        disk.mem("library").file(
            "P.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - team/x.md\n",
        );
        disk.mem("lib2").file("x.md", "x");
        let svc = disk.start_with(ROOT_BASE, vec![agent(&["team/x.md"])]);
        svc.first_scan_join().unwrap();
        assert!(has(&svc, "team/x.md"));
        let changes = svc.numbers().change;

        let removed = svc.remove_mount("team").unwrap();

        assert_eq!(removed.outcome, Removal::Removed);
        assert_eq!(
            removed.now_absent,
            Pointing {
                presets: 1,
                agents: 1
            }
        );
        assert_eq!(row_names(&removed.status), ["library", "engram"]);
        assert_eq!(removed.status.root_generation, ROOT_BASE + 1);
        assert_eq!(svc.numbers().change, changes + 1);
        assert!(!has(&svc, "team/x.md"), "its range left the catalog");
        assert!(
            disk.store.exists(&disk.at("lib2").join("x.md")),
            "no file is touched"
        );
        assert_eq!(
            stored(&disk),
            [("library".to_owned(), "default".to_owned())]
        );
        assert!(
            svc.list(Some(&lp("team")))
                .is_ok_and(|answer| answer.rows.is_empty()),
            "a path in a mount not registered reads as absent"
        );

        let back = svc.add_mount("team", &disk.at("lib2"), false).unwrap();
        assert_eq!(
            back.pointing,
            Pointing {
                presets: 1,
                agents: 1
            },
            "every reference resolves again"
        );
        assert!(has(&svc, "team/x.md"));
    }

    // ADR-0400 decisions 7 and 20
    #[test]
    fn remove_and_relocate_refuse_the_builtin_an_invalid_name_and_a_mount_not_registered() {
        let disk = OnDisk::new(&["library", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "default")]),
        )
        .unwrap();
        let svc = disk.start();
        for answer in [
            svc.remove_mount("Engram").map(drop),
            svc.relocate_mount("engram", &disk.at("lib2"), false)
                .map(drop),
        ] {
            let (class, reason) = refused(answer.unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Conflict, MOUNT_READ_ONLY)
            );
        }
        for answer in [
            svc.remove_mount("a/b").map(drop),
            svc.relocate_mount(".x", &disk.at("lib2"), false).map(drop),
        ] {
            let (class, reason) = refused(answer.unwrap_err());
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::InvalidArgument, MOUNT_NAME_INVALID)
            );
        }
        for answer in [
            svc.remove_mount("nope").map(drop),
            svc.relocate_mount("nope", &disk.at("lib2"), false)
                .map(drop),
        ] {
            let (class, reason) = refused(answer.unwrap_err());
            assert_eq!(
                (class, reason.code.as_str(), reason.args),
                (
                    RefusalClass::NotFound,
                    MOUNT_NOT_FOUND,
                    json!({ "mount": "nope" })
                )
            );
        }
        assert_eq!(svc.numbers().root, ROOT_BASE);
    }

    // ADR-0400 decision 13
    #[test]
    fn relocate_mount_re_points_a_mount_under_its_name_replacing_only_its_range_and_saves_it() {
        let disk = OnDisk::new(&["library", "lib", "lib2", "other"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("team", "lib"), ("extra", "other")]),
        )
        .unwrap();
        disk.mem("lib").file("old.md", "old");
        disk.mem("lib2").file("new.md", "new");
        disk.mem("other").file("x.md", "x");
        let svc = disk.start();
        svc.first_scan_join().unwrap();
        assert!(has(&svc, "team/old.md") && has(&svc, "extra/x.md"));
        let status = svc.relocate_mount("team", &disk.at("lib2"), false).unwrap();
        assert_eq!(
            mounts_of(&svc),
            [
                ("team".to_owned(), disk.at("lib2")),
                ("extra".to_owned(), disk.at("other"))
            ],
            "the name and its place in the list stay"
        );
        assert_eq!(status.root_generation, ROOT_BASE + 1);
        assert!(
            has(&svc, "team/new.md") && !has(&svc, "team/old.md"),
            "the mount's items come from its new folder"
        );
        assert!(has(&svc, "extra/x.md"), "the other mount's items stay");
        assert_eq!(
            stored(&disk),
            [
                ("team".to_owned(), disk.at("lib2").display().to_string()),
                ("extra".to_owned(), disk.at("other").display().to_string())
            ]
        );
        let spelled = disk.temp.path().join("state").join("..").join("library");
        let status = svc.relocate_mount("team", &spelled, false).unwrap();
        assert_eq!(
            word(&disk),
            json!("default"),
            "the default place is stored as the word"
        );
        assert!(mount_row(&status, "team").default);
    }

    // ADR-0381 · ADR-0400 decision 14
    #[test]
    fn a_failed_save_leaves_each_mount_change_live_and_unsaved_and_repeating_the_call_saves_it() {
        let disk = OnDisk::new(&["library", "lib2", "lib3"], "library", None);
        let doc = disk.mounts_doc(&[("library", "default")]);
        std::fs::write(disk.config_path(), &doc).unwrap();
        let svc = disk.start();
        let write_failed = |refusal: LibraryRefusal| {
            let (class, reason) = refused(refusal);
            assert_eq!(
                (class, reason.code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_CONFIG_WRITE_FAILED)
            );
            let why = reason.args["why"].as_str().unwrap().to_owned();
            assert!(
                why.contains("live but not saved") || why.contains("removed but not saved"),
                "{why}"
            );
            assert!(why.contains("disk full"), "{why}");
        };
        let unsaved =
            |svc: &LibraryService| matches!(svc.status().config, ConfigHealth::Unsaved { .. });

        svc.config.fail_writes(Some("disk full"));
        write_failed(svc.add_mount("team", &disk.at("lib2"), false).unwrap_err());
        assert_eq!(
            row_names(&svc.status()),
            ["library", "team", "engram"],
            "the add is live"
        );
        assert!(unsaved(&svc));
        assert_eq!(
            std::fs::read_to_string(disk.config_path()).unwrap(),
            doc,
            "the file keeps its setting"
        );
        assert_eq!(
            row_names(&disk.start().status()),
            ["library", "engram"],
            "the next start follows the file that is left"
        );
        svc.config.fail_writes(None);
        let generation = svc.numbers().root;
        let repeated = svc.add_mount("team", &disk.at("lib2"), false).unwrap();
        assert_eq!(
            repeated.status.config,
            ConfigHealth::Ok,
            "a repeat saves and clears unsaved"
        );
        assert_eq!(
            repeated.status.root_generation, generation,
            "nothing changed but the save"
        );
        assert_eq!(stored(&disk).len(), 2);

        svc.config.fail_writes(Some("disk full"));
        write_failed(
            svc.relocate_mount("team", &disk.at("lib3"), false)
                .unwrap_err(),
        );
        assert_eq!(mount_row(&svc.status(), "team").folder, disk.at("lib3"));
        svc.config.fail_writes(None);
        let generation = svc.numbers().root;
        let repeated = svc.relocate_mount("team", &disk.at("lib3"), false).unwrap();
        assert_eq!(
            (repeated.config, repeated.root_generation),
            (ConfigHealth::Ok, generation)
        );
        assert_eq!(stored(&disk)[1].1, disk.at("lib3").display().to_string());

        svc.config.fail_writes(Some("disk full"));
        write_failed(svc.remove_mount("team").unwrap_err());
        assert_eq!(
            row_names(&svc.status()),
            ["library", "engram"],
            "the removal is live"
        );
        let (class, reason) = refused(svc.remove_mount("teem").unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::NotFound, MOUNT_NOT_FOUND),
            "a mistyped name is still refused while unsaved"
        );
        svc.config.fail_writes(None);
        let repeated = svc.remove_mount("team").unwrap();
        assert_eq!(
            repeated.outcome,
            Removal::SavedOnly,
            "not registered — saved only"
        );
        assert_eq!(repeated.status.config, ConfigHealth::Ok);
        assert_eq!(
            stored(&disk),
            [("library".to_owned(), "default".to_owned())]
        );
        let (class, reason) = refused(svc.remove_mount("team").unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::NotFound, MOUNT_NOT_FOUND),
            "once saved, a repeat is a mount that is not registered"
        );
    }

    #[test]
    fn a_failed_save_over_a_corrupt_file_is_unsaved_not_corrupt_and_the_next_start_follows_the_corrupt_file(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", Some("not json"));
        let svc = disk.start();
        svc.config.fail_writes(Some("disk full"));
        let reason = refused(svc.add_mount("team", &disk.at("lib2"), false).unwrap_err()).1;
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
        assert_eq!(row_names(&status), ["team", "engram"], "the live mounts");
        let next = disk.start().status();
        assert_eq!(
            (next.mounts, next.config),
            (vec![builtin_row()], ConfigHealth::Corrupt)
        );
    }

    /// Waits (at most 10 s) until a mount-change take has found the mutex held.
    fn wait_blocked(svc: &LibraryService) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while svc
            .mount_change_blocked
            .load(std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            assert!(
                Instant::now() < deadline,
                "no mount command waited on the mount-change mutex"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    // ADR-0381 · ADR-0400 decision 16
    #[test]
    fn mount_commands_serialize_across_the_mount_change_and_the_library_json_write() {
        let disk = OnDisk::new(&["lib", "a", "b"], "library", None);
        std::fs::write(disk.config_path(), disk.explicit("lib")).unwrap();
        let svc = disk.start();
        svc.list(Some(&lp("library"))).unwrap();
        let (a, b) = (disk.at("a"), disk.at("b"));
        disk.store.hold_at(At::Start, a.to_str().unwrap());
        let first = {
            let a = a.clone();
            on_thread(&svc, move |svc| svc.relocate_mount("library", &a, false))
        };
        disk.store.wait_held();
        disk.store.hold_at(At::Start, b.to_str().unwrap());
        let second = {
            let b = b.clone();
            on_thread(&svc, move |svc| svc.add_mount("team", &b, false))
        };
        wait_blocked(&svc);
        svc.gate
            .wait_for("only the first mount change is in line", |line, _| {
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
            stored(&disk).len(),
            1,
            "the second walks only after the first's write"
        );
        disk.store.release();
        join(second).unwrap();
        assert_eq!(stored(&disk).len(), 2);
        assert_eq!(mount_row(&svc.status(), "team").folder, b);
    }

    // ADR-0400 decision 10: at load the saved mounts are judged against each other in the boot-scan job.
    #[test]
    fn a_saved_mount_overlapping_an_earlier_one_is_held_unreadable_never_walked_and_blocks_a_launch_until_relocated(
    ) {
        let disk = OnDisk::new(&["lib", "lib/sub", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "lib"), ("inner", "lib/sub")]),
        )
        .unwrap();
        disk.mem("lib").file("a.md", "a");
        disk.mem("lib/sub")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        disk.mem("lib2")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        let svc = disk.start();
        svc.start_boot_scan();
        let deadline = Instant::now() + Duration::from_secs(10);
        while svc.numbers().swap_ins == 0 {
            assert!(Instant::now() < deadline, "the boot scan never applied");
            thread::sleep(Duration::from_millis(2));
        }

        let lib = disk.at("lib");
        assert_eq!(
            disk.store.scans(),
            [scan(lib.to_str().unwrap(), None)],
            "the overlapping mount is not walked"
        );
        let status = svc.status();
        assert_eq!(
            row_names(&status),
            ["library", "inner", "engram"],
            "kept registered"
        );
        let held = &mount_row(&status, "inner").state;
        assert!(
            matches!(held, MountState::Unreadable { why } if why.contains("overlaps the library mount library")),
            "{held:?}"
        );
        assert_eq!(mount_row(&status, "library").state, MountState::Ok);
        let refused_list = svc.list(Some(&lp("inner"))).unwrap_err();
        assert_eq!(
            (refused_list.code.as_str(), &refused_list.args["mount"]),
            (LIBRARY_UNAVAILABLE, &json!("inner")),
            "a reader does not walk it either"
        );
        assert!(
            !svc.refresh(Some(&lp("inner"))).errors.is_empty(),
            "nor a refresh"
        );
        assert_eq!(disk.store.scans().len(), 1);
        let scope = svc.begin_launch();
        let blocked = svc
            .check_in(&scope, &claude_json(&["inner/P.yaml"]))
            .expect_err("a launch reaching it is refused");
        assert!(
            blocked.reasons.iter().any(|reason| matches!(
                reason,
                BlockReason::LibraryUnavailable { reason } if reason.args["mount"] == json!("inner")
            )),
            "{blocked:?}"
        );

        svc.relocate_mount("inner", &disk.at("lib2"), false)
            .unwrap();
        assert_eq!(
            mount_row(&svc.status(), "inner").state,
            MountState::Ok,
            "relocated: no longer held"
        );
        assert!(has(&svc, "inner/P.yaml"));
    }

    /// A disk whose `library.json` saves `library` at `lib` and `inner` at `lib/sub` — `inner` overlaps `library` —
    /// with a preset in each folder and a spare folder `lib2`.
    fn overlapping_disk() -> OnDisk {
        let disk = OnDisk::new(&["lib", "lib/sub", "lib2"], "library", None);
        std::fs::write(
            disk.config_path(),
            disk.mounts_doc(&[("library", "lib"), ("inner", "lib/sub")]),
        )
        .unwrap();
        disk.mem("lib").file("a.md", "a");
        disk.mem("lib/sub")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        disk.mem("lib2")
            .file("P.yaml", "kind: preset\nversion: 1\n");
        disk
    }

    fn is_held(state: &MountState) -> bool {
        matches!(state, MountState::Unreadable { why } if why.contains("overlaps the library mount library"))
    }

    // ADR-0400 decision 10: the load-time judgement runs before the first stat of a mount, whoever takes it —
    //   a launch or a reader before the boot scan's job (here: no job at all) neither walks nor reads the held mount.
    #[test]
    fn a_launch_or_a_reader_before_the_boot_job_judges_the_overlap_first_and_never_walks_the_held_mount(
    ) {
        for launch_first in [true, false] {
            let disk = overlapping_disk();
            let svc = disk.start();
            let launch = |svc: &LibraryService| {
                let scope = svc.begin_launch();
                let blocked = svc
                    .check_in(&scope, &claude_json(&["inner/P.yaml"]))
                    .expect_err("a launch reaching the held mount is refused");
                assert!(
                    blocked.reasons.iter().any(|reason| matches!(
                        reason,
                        BlockReason::LibraryUnavailable { reason }
                            if reason.args["mount"] == json!("inner")
                    )),
                    "launch first: {launch_first}: {blocked:?}"
                );
            };
            if launch_first {
                launch(&svc);
                assert!(
                    disk.store
                        .touched_under(disk.at("lib/sub").to_str().unwrap())
                        .is_empty(),
                    "nothing in the held mount is read"
                );
            }
            let refused = svc.list(Some(&lp("inner"))).unwrap_err();
            assert_eq!(
                refused.args["mount"],
                json!("inner"),
                "launch first: {launch_first}"
            );
            assert_eq!(
                disk.store.scans(),
                [scan(disk.at("lib").to_str().unwrap(), None)],
                "launch first: {launch_first}: the first scan walks only the earlier mount"
            );
            if !launch_first {
                launch(&svc);
            }
            assert!(is_held(&mount_row(&svc.status(), "inner").state));
            assert!(
                disk.store
                    .touched_under(disk.at("lib/sub").to_str().unwrap())
                    .is_empty(),
                "launch first: {launch_first}: nothing in the held mount is read"
            );
        }
    }

    /// Waits (at most 10 s) until `n` callers found the load judgement running and wait for it.
    fn wait_judge_waiters(svc: &LibraryService, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while svc
            .load_judge_blocked
            .load(std::sync::atomic::Ordering::SeqCst)
            < n
        {
            assert!(
                Instant::now() < deadline,
                "fewer than {n} callers waited on the judgement"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    // ADR-0400 decision 10: launches, readers and mount commands arriving while the load-time judgement runs wait for
    //   it — no store access before it ends — and a mount command's change lands after the holds, so a mount removed
    //   and added back meanwhile is not left held. The budgets are long and the door opens only on the test's signal,
    //   so no wall clock decides the order.
    #[test]
    fn launches_readers_and_mount_commands_wait_for_a_running_load_judgement_and_a_mount_added_back_is_not_held(
    ) {
        let disk = overlapping_disk();
        let svc = disk.start_limited(Duration::from_secs(120));
        disk.ops.2.close_at(disk.at("lib/sub"));
        svc.start_boot_scan();
        disk.ops.2.wait_parked();
        let launch = on_thread(&svc, |svc| {
            let scope = svc.begin_launch();
            svc.check_in(&scope, &claude_json(&["library/P.yaml"]))
                .is_ok()
        });
        let reader = on_thread(&svc, |svc| svc.list(Some(&lp("library"))).is_ok());
        wait_judge_waiters(&svc, 2);
        assert!(
            disk.store.scans().is_empty(),
            "nothing walked before the judgement"
        );
        assert!(
            disk.store
                .touched_under(disk.at("lib").to_str().unwrap())
                .is_empty(),
            "nothing read before the judgement"
        );
        let commands = {
            let inner = disk.at("lib/sub");
            on_thread(&svc, move |svc| {
                svc.remove_mount("library")?;
                svc.remove_mount("inner")?;
                svc.add_mount("inner", &inner, false)
            })
        };
        wait_judge_waiters(&svc, 3);
        let names: Vec<String> = mounts_of(&svc).into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            ["library", "inner"],
            "nothing changed while the judgement runs"
        );
        assert!(disk.store.scans().is_empty());
        disk.ops.2.open();
        let added = join(commands).unwrap();
        join(launch);
        join(reader);
        assert_eq!(row_names(&added.status), ["inner", "engram"]);
        assert_eq!(
            mount_row(&added.status, "inner").state,
            MountState::Ok,
            "added back after its hold was ended: not held"
        );
        assert!(has(&svc, "inner/P.yaml"), "walked");
    }

    /// Folder ops on which every ask about a folder at or below `dead` — a share that does not answer — answers only
    /// after `delay`, far past any budget; other folders stat `Ok`, canonicalize to their spelling and have no
    /// identity.
    struct DeadShare {
        dead: PathBuf,
        delay: Duration,
    }

    impl DeadShare {
        fn wait(&self, p: &Path) {
            if p.starts_with(&self.dead) {
                thread::sleep(self.delay);
            }
        }
    }

    impl FolderOps for DeadShare {
        fn stat(&self, p: &Path) -> FolderStatus {
            self.wait(p);
            FolderStatus::Ok
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            self.wait(p);
            Ok(p.to_path_buf())
        }

        fn identity(&self, p: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            self.wait(p);
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        fn read(&self, _: &Path, _: u64) -> io::Result<String> {
            Err(io::Error::other("not modeled"))
        }
    }

    // ADR-0360: the load-time judgement costs the first `status` and the first launch no more than their own budget —
    //   with a dead share among the mounts, each answers within its one budget (before, the judgement's wait came on
    //   top of the caller's own stat: two budgets). The launch's budget is shorter than a read's, so a judgement given
    //   a whole read budget would overrun it. The budgets are long enough that a wrong cost (a second budget, or a
    //   whole read budget for the launch) lies half a budget or more past the bound — scheduling noise does not reach
    //   it.
    #[test]
    fn the_load_judgement_keeps_the_first_status_and_launch_within_their_own_budget() {
        let limit = Duration::from_secs(2);
        for launch_first in [false, true] {
            let ops = DeadShare {
                dead: PathBuf::from("/team"),
                delay: limit * 4,
            };
            let waits = LaunchWaits {
                required: limit / 2,
                advisory: limit / 2,
            };
            let budget = match launch_first {
                false => limit,
                true => waits.required,
            };
            let io = BoundedIo::new(limit, waits, LIST_CAP, Arc::new(ops), Arc::new(ThreadSpawn));
            let (svc, _store) =
                mounted_io(&[("library", "/lib"), ("team", "/team")], Vec::new(), io);
            let began = Instant::now();
            let team = match launch_first {
                false => mount_row(&svc.status(), "team").state.clone(),
                true => {
                    let scope = svc.begin_launch();
                    let blocked = svc
                        .check_in(&scope, &claude_json(&["team/P.yaml"]))
                        .expect_err("the dead mount blocks");
                    let [BlockReason::LibraryUnavailable { reason }] = blocked.reasons.as_slice()
                    else {
                        panic!("{blocked:?}")
                    };
                    MountState::Unreadable {
                        why: reason.args["message"].as_str().unwrap().to_owned(),
                    }
                }
            };
            let took = began.elapsed();
            assert!(
                took < budget + budget / 2,
                "launch first: {launch_first}: one budget ({budget:?}), not more: {took:?}"
            );
            assert!(
                matches!(team, MountState::Unreadable { .. }),
                "launch first: {launch_first}: {team:?}"
            );
        }
    }

    // ADR-0402 decision 5: a first scan that runs the load-time judgement itself, which a dead share makes take its
    //   whole budget, stats its mounts on a fresh budget the judgement charged nothing to, so the healthy mount is
    //   walked and readable and only the dead one — which gave no answer to its own ask — is recorded unreadable.
    #[test]
    fn a_first_scan_that_runs_the_load_judgement_against_a_dead_share_stats_on_a_fresh_budget() {
        let limit = Duration::from_millis(800);
        let ops = DeadShare {
            dead: PathBuf::from("/team"),
            delay: limit * 4,
        };
        let io = BoundedIo::new(
            limit,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(ops),
            Arc::new(ThreadSpawn),
        );
        let (svc, store) = mounted_io(&[("library", "/lib"), ("team", "/team")], Vec::new(), io);
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let library = MountName::parse("library").unwrap();
        let seen = svc.with_read(|state| state.seen(&library));
        assert!(seen.usable(), "the healthy mount is walked: {seen:?}");
        assert!(has(&svc, "library/a.md"));
        let unavailable: Vec<(String, MountState)> = svc
            .folders()
            .unwrap()
            .unavailable
            .into_iter()
            .filter(|left| left.mount != BUILTIN_MOUNT)
            .map(|left| (left.mount, left.state))
            .collect();
        assert_eq!(
            unavailable,
            [(
                "team".to_owned(),
                MountState::Unreadable {
                    why: crate::folder::TIMEOUT.to_owned()
                }
            )],
            "of the library mounts, only the dead one, by its own timeout (the test has no built-in folder)"
        );
        assert_eq!(
            store.scans(),
            [scan("/lib", None)],
            "walked once; the dead mount never"
        );
    }

    /// Folder ops whose stat of a folder listed in `slow` answers `Ok` only after its delay (any other at once); a
    /// folder's canonical path is its spelling, the folder at the door waiting until the door opens; no folder has an
    /// identity.
    struct SlowStats {
        slow: Vec<(PathBuf, Duration)>,
        door: CanonicalDoor,
    }

    impl FolderOps for SlowStats {
        fn stat(&self, p: &Path) -> FolderStatus {
            if let Some((_, delay)) = self.slow.iter().find(|(folder, _)| folder == p) {
                thread::sleep(*delay);
            }
            FolderStatus::Ok
        }

        fn list(&self, _: &Path, _: usize) -> Result<FolderListing, FolderStatus> {
            Err(FolderStatus::Unreadable("not modeled".to_owned()))
        }

        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            self.door.pass(p);
            Ok(p.to_path_buf())
        }

        fn identity(&self, _: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        fn read(&self, _: &Path, _: u64) -> io::Result<String> {
            Err(io::Error::other("not modeled"))
        }
    }

    // ADR-0402 decision 5: a reader's first scan that waits for a running load-time judgement (the boot job's, begun
    //   just before) and wakes with a small part of its budget left still stats its mounts on a fresh budget — the slow
    //   but healthy mount, which answers within one budget and not within that remainder, is walked and readable, and
    //   only the dead one is recorded unreadable, by its own timeout. The remainder is set by the door: it opens about
    //   300 ms before the waiter's budget ends (or the judgement ends on its own budget, which began first), far less
    //   than the healthy mount's 1 s.
    #[test]
    fn a_first_scan_waking_from_the_load_judgement_with_little_budget_left_stats_on_a_fresh_budget()
    {
        let limit = Duration::from_secs(2);
        let ops = Arc::new(SlowStats {
            slow: vec![
                (PathBuf::from("/lib"), limit / 2),
                (PathBuf::from("/team"), limit * 4),
            ],
            door: CanonicalDoor::default(),
        });
        ops.door.close_at(PathBuf::from("/lib"));
        let io = BoundedIo::new(
            limit,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::clone(&ops) as Arc<dyn FolderOps>,
            Arc::new(ThreadSpawn),
        );
        let (svc, store) = mounted_io(&[("library", "/lib"), ("team", "/team")], Vec::new(), io);
        store.mem("/lib").file("a.md", "a");
        // What the boot scan's job runs first, on its own read budget.
        let judge = on_thread(&svc, |svc| svc.judge_at_load(&svc.io.begin_read()));
        ops.door.wait_parked();
        let reader = on_thread(&svc, |svc| svc.first_scan_join());
        wait_judge_waiters(&svc, 1);
        thread::sleep(limit - Duration::from_millis(300));
        ops.door.open();
        assert!(join(judge));
        join(reader).unwrap();
        let library = MountName::parse("library").unwrap();
        let seen = svc.with_read(|state| state.seen(&library));
        assert!(
            seen.usable(),
            "the slow but healthy mount is walked: {seen:?}"
        );
        assert!(has(&svc, "library/a.md"));
        let unavailable: Vec<(String, MountState)> = svc
            .folders()
            .unwrap()
            .unavailable
            .into_iter()
            .filter(|left| left.mount != BUILTIN_MOUNT)
            .map(|left| (left.mount, left.state))
            .collect();
        assert_eq!(
            unavailable,
            [(
                "team".to_owned(),
                MountState::Unreadable {
                    why: crate::folder::TIMEOUT.to_owned()
                }
            )],
            "of the library mounts, only the dead one, by its own timeout"
        );
        assert_eq!(store.scans(), [scan("/lib", None)], "walked once");
    }

    // ADR-0402 decision 5: a first scan whose whole wait for the load-time judgement runs out (here the judgement is
    //   held up by the library lock, not by IO) states no mount — it records nothing, walks nothing and does not raise
    //   the first scan — and goes round to a fresh scan once the judgement is done, which walks both mounts.
    #[test]
    fn a_first_scan_whose_wait_for_the_load_judgement_runs_out_records_nothing_and_goes_round() {
        let limit = Duration::from_millis(300);
        let io = BoundedIo::new(
            limit,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StatOps::default()),
            Arc::new(ThreadSpawn),
        );
        let (svc, store) = mounted_io(&[("library", "/lib"), ("team", "/team")], Vec::new(), io);
        store.mem("/lib").file("a.md", "a");
        let held = svc.gate.lib().write().unwrap();
        let judge = on_thread(&svc, |svc| svc.judge_at_load(&svc.io.begin_read()));
        let deadline = Instant::now() + Duration::from_secs(10);
        while *lock(&svc.load_judgement) != Judgement::Running {
            assert!(Instant::now() < deadline, "the judgement never started");
            thread::sleep(Duration::from_millis(2));
        }
        let reader = on_thread(&svc, |svc| svc.first_scan_join());
        // The reader's wait ran out: it drew its scan's ticket with nothing stated, and its walk waits for the lock.
        svc.gate.wait_for("the reader's scan entered", |line, _| {
            line == [("scan", true)]
        });
        assert_eq!(
            svc.load_judge_blocked
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        drop(held);
        assert!(join(judge));
        join(reader).unwrap();
        for name in ["library", "team"] {
            let mount = MountName::parse(name).unwrap();
            let seen = svc.with_read(|state| state.seen(&mount));
            assert!(seen.usable(), "{name}: walked on the next round: {seen:?}");
        }
        assert!(has(&svc, "library/a.md"));
        let lib_walks = store
            .scans()
            .into_iter()
            .filter(|walk| *walk == scan("/lib", None))
            .count();
        assert_eq!(
            lib_walks, 1,
            "walked once, on the round after the judgement"
        );
    }

    // ADR-0400 decision 10: a held mount relocated to its own folder and judged free is no longer held during its
    //   walk — a scan whose stat falls in that walk is not told the stale hold, so its newer stamp does not record the
    //   hold over the walk: the mount is usable right after, with no read walking it again.
    #[test]
    fn a_scan_during_a_held_mounts_relocation_in_place_does_not_record_the_hold_over_the_walk() {
        let disk = overlapping_disk();
        let svc = disk.start();
        assert!(is_held(&mount_row(&svc.status(), "inner").state));
        svc.remove_mount("library").unwrap();
        let sub = disk.at("lib/sub");
        disk.store.hold_at(At::Start, sub.to_str().unwrap());
        let relocate = {
            let sub = sub.clone();
            on_thread(&svc, move |svc| svc.relocate_mount("inner", &sub, false))
        };
        disk.store.wait_held();
        let scan = on_thread(&svc, |svc| svc.first_scan_join());
        svc.gate.wait_for(
            "the scan stat'ed and queued behind the relocation",
            |line, _| line == [("root", true), ("scan", false)],
        );
        disk.store.release();
        let status = join(relocate).unwrap();
        assert_eq!(mount_row(&status, "inner").state, MountState::Ok);
        join(scan).unwrap();
        let inner = MountName::parse("inner").unwrap();
        let seen = svc.with_read(|state| state.seen(&inner));
        assert!(seen.usable(), "{seen:?}");
    }

    // ADR-0382 decision 1: the first start runs before the daemon answers, so it runs no load-time judgement — not
    //   even over a saved list of overlapping mounts, where it is not due.
    #[test]
    fn the_first_start_runs_no_load_judgement() {
        let disk = overlapping_disk();
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::NotDue);
        assert_eq!(*lock(&svc.load_judgement), Judgement::NotStarted);
        assert!(lock(&svc.overlapping).is_empty(), "nothing judged or held");
        assert_eq!(
            disk.ops.2.passed.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no folder canonicalized"
        );
    }

    // ADR-0400 decisions 10 and 14: a held mount relocated to its own folder is not a repeat — it is judged
    //   again (still overlapping: refused) and, once free, walked and published.
    #[test]
    fn a_held_mount_relocated_to_its_own_folder_is_judged_again_and_walked_once_free() {
        let disk = overlapping_disk();
        let svc = disk.start();
        assert!(is_held(&mount_row(&svc.status(), "inner").state));
        let (class, reason) = refused(
            svc.relocate_mount("inner", &disk.at("lib/sub"), false)
                .unwrap_err(),
        );
        assert_eq!(
            (class, reason.code.as_str(), &reason.args["mount"]),
            (RefusalClass::Conflict, MOUNT_OVERLAPS, &json!("library")),
            "judged again, not saved as a repeat"
        );
        svc.remove_mount("library").unwrap();
        let root = svc.numbers().root;
        let status = svc
            .relocate_mount("inner", &disk.at("lib/sub"), false)
            .unwrap();
        assert_eq!(mount_row(&status, "inner").state, MountState::Ok);
        assert_eq!(status.root_generation, root + 1, "a mount change ran");
        assert!(has(&svc, "inner/P.yaml"), "walked");
    }

    // ADR-0400 decision 10: a hold ends when its mount is removed or relocated — the same name registered
    //   at the same folder again later is not held.
    #[test]
    fn a_hold_ends_when_its_mount_is_removed_or_relocated() {
        let disk = overlapping_disk();
        let svc = disk.start();
        assert!(is_held(&mount_row(&svc.status(), "inner").state));
        svc.remove_mount("inner").unwrap();
        svc.remove_mount("library").unwrap();
        let added = svc.add_mount("inner", &disk.at("lib/sub"), false).unwrap();
        assert_eq!(
            mount_row(&added.status, "inner").state,
            MountState::Ok,
            "removed: the hold ended"
        );

        let disk = overlapping_disk();
        let svc = disk.start();
        assert!(is_held(&mount_row(&svc.status(), "inner").state));
        svc.relocate_mount("inner", &disk.at("lib2"), false)
            .unwrap();
        svc.remove_mount("library").unwrap();
        let status = svc
            .relocate_mount("inner", &disk.at("lib/sub"), false)
            .unwrap();
        assert_eq!(
            mount_row(&status, "inner").state,
            MountState::Ok,
            "relocated away and back: the hold ended"
        );
    }

    // ADR-0400 decision 10 · ADR-0382 decision 4: a scan walks a mount whose stat worker could not be started,
    //   but never a held one.
    #[test]
    fn a_held_mount_is_not_walked_when_its_stat_worker_cannot_be_started() {
        let (svc, store) = mounted_spawn(
            &[("library", "/lib"), ("inner", "/lib")],
            Arc::new(StatOps::default()),
            Vec::new(),
            Arc::new(FailingSpawn),
        );
        svc.first_scan_join().unwrap();
        assert_eq!(
            store.scans(),
            [scan("/lib", None)],
            "the earlier mount walked on the caller's thread, the held one not"
        );
        let inner = MountName::parse("inner").unwrap();
        assert!(is_held(&svc.with_read(|state| state.seen(&inner).shown())));
    }

    // ADR-0400 decision 10: two mounts on one folder keep their own stat in a read computation — the held one
    //   blocks with its row and the other resolves, in either order.
    #[test]
    fn two_mounts_on_one_folder_are_judged_each_as_itself_in_either_order() {
        for presets in [
            ["library/P.yaml", "inner/P.yaml"],
            ["inner/P.yaml", "library/P.yaml"],
        ] {
            let profile = claude_json(&presets);
            let (svc, store) = mounted_agents(
                &[("library", "/lib"), ("inner", "/lib")],
                Arc::new(StatOps::default()),
                vec![profile.clone()],
            );
            store.mem("/lib").file(
                "P.yaml",
                "kind: preset\nversion: 1\ninstructions:\n  - library/a.md\n",
            );
            store.mem("/lib").file("a.md", "a");
            let resolution = svc.resolve_agent(profile.id).expect("a claude agent");
            assert_eq!(
                resolution.problems,
                [library_unavailable(
                    "inner",
                    "overlaps the library mount library; relocate or remove it"
                )],
                "{presets:?}"
            );
            assert_eq!(resolution.instructions[0].0, "library/a.md", "{presets:?}");
        }
    }

    // ── first start ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_first_start_creates_the_default_place_writes_the_default_value_and_publishes_it() {
        let disk = OnDisk::new(&["default_library"], "default_library", None);
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::Published);
        assert!(disk.at("default_library").is_dir());
        assert_eq!(
            disk.config(),
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "default_library", "path": "default" }] })
            ),
            "the mount `default_library` at the default value, not the path"
        );
        assert_eq!(
            mounts_of(&svc),
            [("default_library".to_owned(), disk.at("default_library"))],
            "one mount published"
        );
        let status = svc.status();
        assert_eq!(
            (status.mounts, status.config),
            (
                vec![
                    MountRow {
                        name: "default_library".to_owned(),
                        folder: disk.at("default_library"),
                        default: true,
                        builtin: false,
                        state: MountState::Ok,
                    },
                    builtin_row(),
                ],
                ConfigHealth::Ok
            )
        );
        assert!(svc
            .list(Some(&lp("default_library")))
            .unwrap()
            .rows
            .is_empty());
        assert_eq!(svc.first_start(), FirstStart::NotDue, "once");
        assert_eq!(
            disk.start().first_start(),
            FirstStart::NotDue,
            "the next start"
        );
    }

    #[test]
    fn an_uncreatable_default_place_leaves_no_mount_and_no_file_and_the_next_start_retries() {
        let disk = OnDisk::new(
            &["blocker/default_library"],
            "blocker/default_library",
            None,
        );
        std::fs::write(disk.temp.path().join("blocker"), "a file").unwrap();
        let svc = disk.start();
        assert!(matches!(svc.first_start(), FirstStart::Failed(_)));
        assert!(disk.config().is_none());
        let status = svc.status();
        assert_eq!(
            (status.mounts, status.config),
            (vec![builtin_row()], ConfigHealth::Ok)
        );
        std::fs::remove_file(disk.temp.path().join("blocker")).unwrap();
        let next = disk.start();
        assert_eq!(next.first_start(), FirstStart::Published);
        assert_eq!(
            mount_row(&next.status(), "default_library").folder,
            disk.at("blocker/default_library")
        );
    }

    #[test]
    fn a_first_start_whose_stat_is_refused_publishes_and_writes_nothing_and_the_next_start_retries()
    {
        let disk = OnDisk::new(&["library"], "library", None);
        lock(&disk.ops.0).insert(
            disk.at("library"),
            FolderStatus::Unreadable("denied".to_owned()),
        );
        let svc = disk.start();
        assert!(matches!(svc.first_start(), FirstStart::Failed(why) if why.contains("denied")));
        assert!(disk.config().is_none());
        assert_eq!(svc.status().mounts, [builtin_row()]);
        assert_eq!(svc.numbers().root, ROOT_BASE);
        assert!(disk.store.scans().is_empty(), "nothing walked");
        lock(&disk.ops.0).clear();
        assert_eq!(disk.start().first_start(), FirstStart::Published);
    }

    #[test]
    fn a_first_start_whose_write_fails_keeps_the_default_mount_live_unsaved_and_the_next_start_runs_it_again(
    ) {
        let disk = OnDisk::new(&["default_library"], "default_library", None);
        let svc = disk.start();
        svc.config.fail_writes(Some("disk full"));
        assert!(matches!(svc.first_start(), FirstStart::Unsaved(why) if why.contains("disk full")));
        assert!(disk.config().is_none(), "no file");
        let status = svc.status();
        let library = mount_row(&status, "default_library");
        assert_eq!(
            (&library.folder, library.default, &library.state),
            (&disk.at("default_library"), true, &MountState::Ok),
            "the default mount is live this run"
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
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "default_library", "path": "default" }] })
            )
        );
        assert_eq!(
            mount_row(&next.status(), "default_library").folder,
            disk.at("default_library")
        );
    }

    #[test]
    fn a_first_start_that_finds_a_newer_file_at_its_write_stays_live_and_the_next_start_is_read_only(
    ) {
        let disk = OnDisk::new(&["default_library", "lib2"], "default_library", None);
        let svc = disk.start();
        let newer = r#"{"schema_version": 9, "mounts": []}"#;
        std::fs::write(disk.config_path(), newer).unwrap();
        assert!(matches!(svc.first_start(), FirstStart::Unsaved(why) if why.contains("newer")));
        assert_eq!(std::fs::read_to_string(disk.config_path()).unwrap(), newer);
        assert_eq!(
            mount_row(&svc.status(), "default_library").folder,
            disk.at("default_library")
        );
        let next = disk.start();
        assert_eq!(next.first_start(), FirstStart::NotDue);
        let status = next.status();
        assert_eq!(status.mounts, [builtin_row()]);
        assert!(
            matches!(&status.config, ConfigHealth::ReadOnly { .. }),
            "{status:?}"
        );
        let reason = refused(next.add_mount("team", &disk.at("lib2"), false).unwrap_err()).1;
        assert_eq!(reason.code, LIBRARY_CONFIG_READ_ONLY);
    }

    #[test]
    fn a_first_start_over_files_already_in_the_default_place_publishes_without_a_walk_and_the_boot_scan_fills_the_catalog(
    ) {
        let disk = OnDisk::new(&["default_library"], "default_library", None);
        disk.mem("default_library").file("a.md", "a");
        let root = disk.at("default_library");
        let root_text = root.to_str().unwrap();
        disk.store.hold_at(At::Start, root_text);
        let svc = disk.start();
        assert_eq!(
            join(on_thread(&svc, |svc| svc.first_start())),
            FirstStart::Published,
            "returned with a walk of the default place armed to park"
        );
        assert!(
            disk.store.scans().is_empty(),
            "the first start walked nothing"
        );
        assert_eq!(
            disk.config(),
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "default_library", "path": "default" }] })
            )
        );
        assert_eq!(root_of(&svc), Some(root.clone()));
        let numbers = svc.numbers();
        assert_eq!((numbers.root, numbers.swap_ins), (ROOT_BASE + 1, 0));

        svc.start_boot_scan();
        disk.store.wait_held();
        assert!(
            !has(&svc, "default_library/a.md"),
            "the boot-scan method returned with its walk parked"
        );
        let reader = on_thread(&svc, |svc| svc.list(Some(&lp("default_library"))).unwrap());
        svc.gate.wait_for(
            "the reader waits at the first-scan entrance",
            |_, waiting| waiting == 1,
        );
        disk.store.release();
        assert_eq!(paths(&join(reader)), ["default_library/a.md"]);
        assert_eq!(
            disk.store.scans(),
            [scan(root_text, None)],
            "one walk: the boot scan's"
        );
        assert_eq!(svc.numbers().swap_ins, 1);
    }

    #[test]
    fn a_walk_that_fails_after_a_published_first_start_leaves_library_json_written_and_readers_unavailable_until_a_scan_succeeds(
    ) {
        let disk = OnDisk::new(&["default_library"], "default_library", None);
        disk.mem("default_library").file("a.md", "a");
        let root = disk.at("default_library");
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::Published);
        let written = disk.config();
        *lock(&disk.store.fail_next_scan) = true;
        disk.store.hold_at(At::Start, root.to_str().unwrap());
        svc.start_boot_scan();
        disk.store.wait_held();
        disk.store.release();
        svc.gate
            .wait_for("the failed boot scan ended", |line, _| line.is_empty());
        assert_eq!(
            disk.config(),
            written,
            "library.json stays written after the walk failed"
        );
        assert_eq!(
            root_of(&svc),
            Some(root.clone()),
            "the mount stays published"
        );
        assert_eq!(
            mount_row(&svc.status(), "default_library").state,
            MountState::Ok,
            "status answers the mount's stat"
        );

        *lock(&disk.store.fail_next_scan) = true;
        let refused_list = svc.list(Some(&lp("default_library"))).unwrap_err();
        assert_eq!(
            (refused_list.code.as_str(), &refused_list.args["mount"]),
            (LIBRARY_UNAVAILABLE, &json!("default_library")),
            "the reader walked again and that walk failed too"
        );
        let partial = svc.folders().unwrap();
        assert_eq!(
            partial.unavailable,
            [
                UnavailableMount {
                    mount: MountName::first().as_str().to_owned(),
                    state: MountState::Unreadable {
                        why: io::Error::from(io::ErrorKind::PermissionDenied).to_string()
                    }
                },
                UnavailableMount {
                    mount: BUILTIN_MOUNT.to_owned(),
                    state: no_builtin_folder(),
                },
            ],
            "a whole-tree read answers partially"
        );
        lock(&disk.ops.0).insert(root.clone(), FolderStatus::Missing);
        assert_eq!(
            settled(&svc, "default_library", &MountState::Ok),
            MountState::Missing {
                why: "missing".to_owned()
            }
        );
        lock(&disk.ops.0).clear();

        assert_eq!(
            paths(&svc.list(Some(&lp("default_library"))).unwrap()),
            ["default_library/a.md"],
            "a scan succeeded"
        );
        assert_eq!(disk.store.scans().len(), 3);
        assert_eq!(disk.config(), written);
    }

    #[test]
    fn a_configured_mount_folder_that_vanished_is_missing_not_recreated_and_the_setting_is_unchanged(
    ) {
        for root in ["library", "chosen"] {
            let disk = OnDisk::new(&["library", "chosen"], "library", None);
            let config = match root {
                "library" => {
                    r#"{"schema_version": 2, "mounts": [{"name": "library", "path": "default"}]}"#
                        .to_owned()
                }
                _ => disk.explicit(root),
            };
            std::fs::write(disk.config_path(), &config).unwrap();
            lock(&disk.ops.0).insert(disk.at(root), FolderStatus::Missing);
            let svc = disk.start();
            assert_eq!(svc.first_start(), FirstStart::NotDue, "{root}");
            let status = svc.status();
            assert_eq!(
                mount_row(&status, "library"),
                &MountRow {
                    name: "library".to_owned(),
                    folder: disk.at(root),
                    default: root == "library",
                    builtin: false,
                    state: MountState::Missing {
                        why: "missing".to_owned()
                    },
                },
                "{root}"
            );
            assert!(!disk.at(root).exists(), "{root}: never re-created");
            assert_eq!(
                std::fs::read_to_string(disk.config_path()).unwrap(),
                config,
                "{root}"
            );
        }
    }

    #[test]
    fn a_corrupt_config_is_no_first_start_and_a_mount_added_over_it_copies_it_aside_and_makes_it_ok(
    ) {
        let disk = OnDisk::new(&["library", "lib2"], "library", Some("not json"));
        let svc = disk.start();
        assert_eq!(svc.first_start(), FirstStart::NotDue);
        let status = svc.status();
        assert_eq!(
            (status.mounts, status.config),
            (vec![builtin_row()], ConfigHealth::Corrupt)
        );
        let added = svc.add_mount("library", &disk.at("lib2"), false).unwrap();
        assert_eq!(added.status.config, ConfigHealth::Ok);
        assert_eq!(
            std::fs::read_to_string(disk.state().join("library.json.corrupt")).unwrap(),
            "not json"
        );
        assert_eq!(
            stored(&disk),
            [("library".to_owned(), disk.at("lib2").display().to_string())]
        );
    }

    // ── the boot scan ────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_boot_scan_also_runs_on_a_normal_start_and_fills_the_catalog_with_no_reader() {
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file("a.md", "a");
        let spawn = Arc::new(JoinSpawn::default());
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        assert_eq!(svc.first_start(), FirstStart::NotDue);
        svc.start_boot_scan();
        spawn.join_all();
        assert_eq!(store.scans(), [scan("/lib", None)]);
        assert_eq!(svc.numbers().swap_ins, 1);
        assert!(has(&svc, "library/a.md"));
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/a.md"]
        );
        assert_eq!(
            store.scans().len(),
            1,
            "the reader found the first scan raised"
        );
    }

    #[test]
    fn the_boot_scan_starts_no_job_without_a_published_root() {
        let store = TestStore::new(&["/lib"]);
        let spawn = Arc::new(JoinSpawn::default());
        let svc = service(
            None,
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        svc.start_boot_scan();
        assert!(lock(&spawn.0).is_empty(), "no job was started");
        assert!(store.scans().is_empty());
    }

    #[test]
    fn a_boot_scan_whose_job_cannot_start_leaves_the_first_scan_to_the_first_reader() {
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file("a.md", "a");
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::new(FailingSpawn),
        );
        svc.start_boot_scan();
        assert!(store.scans().is_empty(), "not walked in place");
        assert_eq!(svc.numbers().swap_ins, 0);
        assert_eq!(
            paths(&svc.list(Some(&lp("library"))).unwrap()),
            ["library/a.md"]
        );
        assert_eq!(store.scans(), [scan("/lib", None)]);
    }

    #[test]
    fn the_boot_scan_joins_a_first_scan_a_reader_already_started_instead_of_walking_again() {
        let store = TestStore::new(&["/lib"]);
        store.mem("/lib").file("a.md", "a");
        let spawn = Arc::new(JoinSpawn::default());
        let svc = service(
            Some("/lib"),
            &store,
            Vec::new(),
            Arc::new(StatOps::default()),
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        store.hold_at(At::Start, "/lib");
        let reader = on_thread(&svc, |svc| svc.list(Some(&lp("library"))).unwrap());
        store.wait_held();
        svc.start_boot_scan();
        svc.gate.wait_for(
            "the boot scan waits on the reader's walk",
            |line, waiting| line == [("scan", true)] && waiting == 1,
        );
        store.release();
        assert_eq!(paths(&join(reader)), ["library/a.md"]);
        spawn.join_all();
        assert_eq!(store.scans(), [scan("/lib", None)]);
    }

    // ── the root generation on a change ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_stale_root_generation_is_refused_inside_the_changes_own_lock_take_even_when_queued_behind_a_mount_change(
    ) {
        let (svc, store) = service_on(&["/lib", "/lib2"], Some("/lib"));
        svc.list(Some(&lp("library"))).unwrap();
        let given = svc.status().root_generation;
        store.hold_at(At::Start, "/lib2");
        let root_change = on_thread(&svc, |svc| relocate(svc, "library", "/lib2"));
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
            .wait_for("the edit is queued behind the mount change", |line, _| {
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
        let root = svc.change_at(Some(current), |_, mounts, _| {
            let first = mounts.iter().next().map(|first| first.folder.clone());
            (Ok::<_, LibraryRefusal>(first), Vec::new())
        });
        assert_eq!(root.unwrap().unwrap(), Some(PathBuf::from("/lib2")));
        assert!(svc.change_at(None, |_, _, _| ((), Vec::new())).is_ok());
        assert_eq!(svc.numbers().change, changes + 2);
    }

    #[test]
    fn a_generation_kept_from_an_earlier_run_with_another_base_is_refused() {
        let disk = OnDisk::new(&["lib", "lib2"], "library", None);
        std::fs::write(disk.config_path(), disk.explicit("lib")).unwrap();
        let first = disk.start_at(ROOT_BASE);
        let kept = first
            .relocate_mount("library", &disk.at("lib2"), false)
            .unwrap()
            .root_generation;
        let next = disk.start_at(ROOT_BASE + 50_000);
        assert_eq!(next.status().root_generation, ROOT_BASE + 50_000);
        let moved = next
            .relocate_mount("library", &disk.at("lib"), false)
            .unwrap()
            .root_generation;
        assert_eq!(moved, ROOT_BASE + 50_001, "one mount change in each run");
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

    // A change refused inside its body applied nothing, so it moves no number; one that applied a catalog item does.
    #[test]
    fn a_change_refused_inside_its_body_moves_no_number_unless_it_applied_a_catalog_item() {
        let (svc, store) = library();
        store.mem("/lib").file("a.md", "a");
        svc.first_scan_join().unwrap();
        let before = svc.numbers();
        let refused_change = svc.change_at(None, |_, _, _| {
            (
                Err::<(), _>(LibraryRefusal::unavailable("no".to_owned())),
                Vec::new(),
            )
        });
        assert!(refused_change.unwrap().is_err());
        assert_eq!(
            svc.numbers(),
            before,
            "a refusal that applied nothing moves nothing"
        );
        let touched = svc.change_at(None, |_, _, _| {
            (
                Err::<(), _>(LibraryRefusal::unavailable("no".to_owned())),
                vec![key(&lp("library/a.md"))],
            )
        });
        assert!(touched.unwrap().is_err());
        assert_eq!(
            svc.numbers().change,
            before.change + 1,
            "it applied an item"
        );
        let applied = svc.change_at(None, |_, _, _| (Ok::<(), LibraryRefusal>(()), Vec::new()));
        assert!(applied.unwrap().is_ok());
        assert_eq!(
            svc.numbers().change,
            before.change + 2,
            "an answer that is not a refusal moves it"
        );
        assert_eq!(
            svc.numbers().root,
            before.root,
            "a change never moves the root generation"
        );
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
                ("engram", "engram", false),
                ("engram/settings", "settings", false),
                ("library", "library", false),
                ("library/deep", "deep", false),
                ("library/deep/er", "er", false),
                ("library/rules", "rules", false),
                ("library/skills", "skills", false),
                ("library/skills/s", "s", true),
            ]
        );
    }

    // ADR-0400 decision 4: with no mount registered the tree holds only the built-in mount and a path names no mount —
    //   absent.
    #[test]
    fn folders_and_read_with_no_mount_answer_only_the_builtin_mount_and_not_found() {
        let (svc, _store) = service_on(&["/lib"], None);
        let tree: Vec<String> = svc
            .folders()
            .unwrap()
            .folders
            .into_iter()
            .map(|node| node.path)
            .collect();
        assert_eq!(tree, ["engram", "engram/settings"]);
        let (class, reason) = refused(svc.read(&lp("library/a.md")).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::NotFound, NOT_FOUND)
        );
    }

    #[test]
    fn read_answers_the_row_absolute_path_direct_and_indirect_references_excerpt_and_settings_fields(
    ) {
        let store = TestStore::new(&["/lib"]);
        let top_agent = agent(&["library/top.yaml"]);
        let mut inner_agent = agent(&["library/inner.yaml"]);
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
            "kind: preset\nversion: 1\ninstructions:\n  - library/rules/a.md\n",
        );
        mem.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - library/inner.yaml\n",
        );
        mem.file(
            "set.yaml",
            "kind: settings\nversion: 1\nclaude: {}\ncodex: {}\n",
        );

        let a = svc.read(&lp("library/rules/a.md")).unwrap();
        assert_eq!(a.row.path, "library/rules/a.md");
        assert_eq!(
            (a.row.kind.clone(), a.row.references),
            (ItemKind::Instruction, 2)
        );
        assert_eq!(a.absolute, Path::new("/lib").join("rules").join("a.md"));
        assert_eq!(a.presets_direct, ["library/inner.yaml"]);
        assert_eq!(a.presets_indirect, ["library/top.yaml"]);
        assert!(a.agents_direct.is_empty());
        let mut both = vec![top_agent.id, inner_agent.id];
        both.sort_unstable();
        assert_eq!(a.agents_indirect, both);
        assert_eq!(a.excerpt, Some(excerpt(rules)));
        assert!(a.settings_fields.is_empty());

        let inner = svc.read(&lp("library/inner.yaml")).unwrap();
        assert_eq!(inner.presets_direct, ["library/top.yaml"]);
        assert!(inner.presets_indirect.is_empty());
        assert_eq!(inner.agents_direct, [inner_agent.id]);
        assert_eq!(inner.agents_indirect, [top_agent.id]);
        assert_eq!(inner.row.references, 2);
        assert_eq!(inner.excerpt, None);

        let settings = svc.read(&lp("library/set.yaml")).unwrap();
        assert_eq!(settings.settings_fields, ["claude", "codex"]);

        let (class, reason) = refused(svc.read(&lp("library/missing.md")).unwrap_err());
        assert_eq!(
            (class, reason.code.as_str()),
            (RefusalClass::NotFound, NOT_FOUND)
        );
        assert_eq!(reason.args, json!({ "path": "library/missing.md" }));
    }

    // ── mount rename (ADR-0400 decisions 12, 14 and 18) ─────────────────────────────────────────────────

    mod rename_mount {
        use std::sync::{mpsc, Weak};

        use engram_dashboard_agent::profile::{Refusal, StoreStatus};
        use engram_dashboard_agent::profile_edit::{
            Attachments, PresetPathRename, ProfileEditError, ProfileEditPort, WorkspaceLock,
        };

        use super::*;
        use crate::edit::{AddEntries, NewItem};

        fn preset(body: &str) -> String {
            format!("kind: preset\nversion: 1\n{body}")
        }

        /// A profile-edit port over the service's registry: attachments and path rewrites go through the registry's
        /// public update. `fail` = the next rewrite refuses with it; `entered` is told when a rewrite starts, and a
        /// rewrite then waits for `release` (when set).
        struct RegistryPort {
            registry: Arc<ProfileRegistry>,
            fail: Mutex<Option<ProfileEditError>>,
            rewrites: Mutex<Vec<Vec<PresetPathRename>>>,
            entered: Mutex<Option<mpsc::Sender<()>>>,
            release: Mutex<Option<mpsc::Receiver<()>>>,
        }

        impl ProfileEditPort for RegistryPort {
            fn set_attachments(
                &self,
                id: AgentId,
                next: Attachments,
            ) -> Result<(), ProfileEditError> {
                let present = self.registry.try_update_with(id, |profile| {
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
                renames: &[PresetPathRename],
            ) -> Result<Vec<AgentId>, ProfileEditError> {
                if let Some(entered) = lock(&self.entered).take() {
                    entered.send(()).unwrap();
                }
                if let Some(release) = lock(&self.release).take() {
                    release
                        .recv_timeout(Duration::from_secs(20))
                        .expect("the test releases the agents pass");
                }
                lock(&self.rewrites).push(renames.to_vec());
                if let Some(failure) = lock(&self.fail).take() {
                    return Err(failure);
                }
                let mut changed = Vec::new();
                for profile in self.registry.list() {
                    let next: Vec<AttachedPreset> = profile
                        .presets
                        .iter()
                        .map(|attached| {
                            match renames.iter().find(|rename| {
                                same_path(Path::new(&rename.from), Path::new(&attached.path))
                            }) {
                                Some(rename) => AttachedPreset {
                                    path: rename.to.clone(),
                                },
                                None => attached.clone(),
                            }
                        })
                        .collect();
                    if next != profile.presets {
                        self.registry
                            .try_update_with(profile.id, |stored| stored.presets = next)?;
                        changed.push(profile.id);
                    }
                }
                Ok(changed)
            }

            fn remove_preset_paths(&self, _: &[String]) -> Result<Vec<AgentId>, ProfileEditError> {
                unreachable!("not a rename's call")
            }

            fn workspace_lock(
                &self,
                _: AgentId,
            ) -> Result<Option<WorkspaceLock>, ProfileEditError> {
                Ok(None)
            }
        }

        /// A registry port plugged into `svc`; keep it alive for the test.
        fn plugged(svc: &LibraryService) -> Arc<RegistryPort> {
            let port = Arc::new(RegistryPort {
                registry: Arc::clone(&svc.values.registry),
                fail: Mutex::new(None),
                rewrites: Mutex::new(Vec::new()),
                entered: Mutex::new(None),
                release: Mutex::new(None),
            });
            let dyn_port: Arc<dyn ProfileEditPort> = Arc::clone(&port) as Arc<dyn ProfileEditPort>;
            let weak: Weak<dyn ProfileEditPort> = Arc::downgrade(&dyn_port);
            svc.plug_profile_edit(weak);
            port
        }

        /// A store whose `agents.json` is newer than this build — read-only (ADR-0291).
        struct NewerAgents(Vec<AgentProfile>);

        impl ProfileStore for NewerAgents {
            fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
                Err(StoreError::Refused(Refusal::Newer { found: 9 }))
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.0.clone()
            }
            fn status(&self) -> StoreStatus {
                StoreStatus::ReadOnly(Refusal::Newer { found: 9 })
            }
        }

        /// Two mounts — `library` at the default place (`lib`, holding `b.md`) and `team` at `lib2` (holding `a.md`).
        fn two_mounts() -> OnDisk {
            let disk = OnDisk::new(&["lib", "lib2"], "lib", None);
            std::fs::write(
                disk.config_path(),
                disk.mounts_doc(&[("library", "default"), ("team", "lib2")]),
            )
            .unwrap();
            disk.mem("lib").file("b.md", "b");
            disk.mem("lib2").file("a.md", "a");
            disk
        }

        /// [`two_mounts`] with `library/P.yaml` and `team/Q.yaml` listing `team/a.md`.
        fn referencing() -> OnDisk {
            let disk = two_mounts();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            disk.mem("lib2").file(
                "Q.yaml",
                &preset("instructions:\n- team/a.md\n- library/b.md\n"),
            );
            disk
        }

        fn started(svc: Arc<LibraryService>) -> Arc<LibraryService> {
            svc.first_scan_join().unwrap();
            svc
        }

        /// The text of the file `rel` in the test folder `folder`.
        fn text(disk: &OnDisk, folder: &str, rel: &str) -> String {
            let bytes = disk
                .store
                .read(&disk.at(folder).join(rel), LIBRARY_READ_CAP)
                .unwrap();
            String::from_utf8(bytes).unwrap()
        }

        fn presets_of(svc: &LibraryService, id: AgentId) -> Vec<String> {
            svc.values
                .registry
                .get(id)
                .unwrap()
                .presets
                .into_iter()
                .map(|attached| attached.path)
                .collect()
        }

        fn code_of(refusal: LibraryRefusal) -> (RefusalClass, String, serde_json::Value) {
            let (class, reason) = refused(refusal);
            (class, reason.code, reason.args)
        }

        /// The `plan_digest` of the plan of renaming `mount` to `to` as the library stands now — what a caller that
        /// read that plan confirms with.
        fn digest(svc: &LibraryService, mount: &str, to: &str) -> String {
            svc.rename_plan(
                &svc.io.begin_read(),
                &MountName::parse(mount).unwrap(),
                &MountName::parse(to).unwrap(),
            )
            .digest()
        }

        // ADR-0400 decision 12: with no file to rewrite no confirmation is asked — the mount takes the name, its items
        //   move to it in the catalog with their state, both numbers move once and library.json names it.
        #[test]
        fn a_rename_with_no_file_to_rewrite_renames_rekeys_and_saves_without_yes() {
            let disk = two_mounts();
            let svc = started(disk.start());
            let (root, change) = (svc.numbers().root, svc.change_number());

            let answer = svc
                .rename_mount("team", "shared", false, None, None)
                .unwrap();

            assert_eq!(row_names(&answer.status), ["library", "shared", "engram"]);
            assert_eq!(answer.status.root_generation, root + 1);
            assert_eq!(svc.change_number(), change + 1);
            assert!(answer.written.is_empty() && answer.stopped.is_none());
            assert!(has(&svc, "shared/a.md") && !has(&svc, "team/a.md"));
            let shared = MountName::parse("shared").unwrap();
            assert!(svc.with_read(|state| state.seen(&shared)).usable());
            assert_eq!(
                stored(&disk),
                [
                    ("library".to_owned(), "default".to_owned()),
                    ("shared".to_owned(), disk.at("lib2").display().to_string()),
                ]
            );
        }

        // ADR-0400 decision 12: a rename that rewrites files applies nothing until `yes` comes with the plan's root
        //   generation — without it the plan, a stale one the plan again — and then rewrites every referencing file,
        //   the renamed mount's own included, whole, and the agents attached in it. The plan names those agents and,
        //   as the answer, what already pointed at the new name.
        #[test]
        fn a_rename_that_rewrites_files_needs_yes_and_the_plans_root_generation() {
            let disk = referencing();
            // Already pointing at `shared`: one reference in another mount's preset and one agent.
            disk.mem("lib")
                .file("R.yaml", &preset("instructions:\n- shared/a.md\n"));
            let (attached, pointing) = (agent(&["team/Q.yaml"]), agent(&["shared/X.yaml"]));
            let attached_id = attached.id;
            let svc = started(disk.start_with(ROOT_BASE, vec![attached, pointing]));
            let _port = plugged(&svc);
            let attached_name = svc
                .values
                .registry
                .get(attached_id)
                .unwrap()
                .canonical_name_when_live();
            let root = svc.numbers().root;

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(args["root_generation"], json!(root));
            let files: Vec<(String, serde_json::Value)> = args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| {
                    (
                        mount["mount"].as_str().unwrap().to_owned(),
                        mount["paths"].clone(),
                    )
                })
                .collect();
            assert_eq!(
                files,
                [
                    ("library".to_owned(), json!(["library/P.yaml"])),
                    ("team".to_owned(), json!(["team/Q.yaml"])),
                ]
            );
            assert_eq!(args["plan"]["whole_files"], json!(true));
            assert_eq!(
                args["plan"]["agents"],
                json!([{ "id": attached_id.to_string(), "name": attached_name }])
            );
            assert_eq!(
                args["plan"]["already_pointing"],
                json!({ "presets": 1, "agents": 1 })
            );
            let (_, code, _) = code_of(
                svc.rename_mount("team", "shared", true, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                code, MOUNT_RENAME_UNCONFIRMED,
                "yes without the root generation"
            );
            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root + 5), None)
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, ROOT_GENERATION_STALE)
            );
            assert_eq!(args["current"], json!(root));
            assert!(args["plan"]["files"].is_array(), "a fresh plan: {args}");
            assert_eq!(svc.numbers().root, root, "nothing applied");
            assert!(text(&disk, "lib", "P.yaml").contains("team/a.md"));

            let answer = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();

            assert_eq!(answer.written, ["library/P.yaml", "shared/Q.yaml"]);
            assert_eq!(
                answer.fixed,
                [
                    ("team/a.md".to_owned(), "shared/a.md".to_owned()),
                    ("team/Q.yaml".to_owned(), "shared/Q.yaml".to_owned()),
                ]
            );
            assert!(answer.stopped.is_none());
            assert_eq!(
                answer.already_pointing,
                Pointing {
                    presets: 1,
                    agents: 1
                }
            );
            assert_eq!(presets_of(&svc, attached_id), ["shared/Q.yaml"]);
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- shared/a.md\n")
            );
            let q = text(&disk, "lib2", "Q.yaml");
            assert!(
                q.contains("- shared/a.md") && q.contains("- library/b.md") && !q.contains("team/"),
                "{q}"
            );
            assert!(has(&svc, "shared/Q.yaml") && has(&svc, "shared/a.md"));
            assert_eq!(row_names(&answer.status), ["library", "shared", "engram"]);
        }

        // ADR-0400 decisions 12 and 24: the plan names, per mount with files to rewrite, whether its folder is
        //   in a git working tree — a `.git` folder or file in it, or in a parent (a mount in a repository's subfolder)
        //   — and the mounts that cannot be read; a mount never walked is a skipped row at its top.
        #[test]
        fn the_plan_names_git_working_trees_and_the_mounts_it_could_not_read() {
            let disk = OnDisk::new(&["lib", "lib2", "repo/sub"], "lib", None);
            std::fs::write(
                disk.config_path(),
                disk.mounts_doc(&[
                    ("library", "default"),
                    ("team", "lib2"),
                    ("sub", "repo/sub"),
                    ("dead", "gone"),
                ]),
            )
            .unwrap();
            {
                let mut table = lock(&disk.ops.0);
                for folder in ["lib", "lib2", "repo/sub"] {
                    for dir in disk.at(folder).ancestors() {
                        table.insert(dir.join(".git"), FolderStatus::Missing);
                    }
                }
                table.insert(disk.at("lib2").join(".git"), FolderStatus::NotADirectory);
                table.insert(disk.at("repo").join(".git"), FolderStatus::Ok);
                table.insert(
                    disk.at("gone"),
                    FolderStatus::Unreadable("access is denied".to_owned()),
                );
            }
            disk.mem("lib2").file("a.md", "a");
            for folder in ["lib", "lib2", "repo/sub"] {
                disk.mem(folder)
                    .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            }
            let svc = started(disk.start());

            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            let git: Vec<(String, serde_json::Value)> = args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| {
                    (
                        mount["mount"].as_str().unwrap().to_owned(),
                        mount["git"].clone(),
                    )
                })
                .collect();
            assert_eq!(
                git,
                [
                    ("library".to_owned(), json!(false)),
                    ("team".to_owned(), json!(true)),
                    ("sub".to_owned(), json!(true)),
                ]
            );
            assert_eq!(
                args["plan"]["unread"],
                json!([{
                    "mount": "dead", "state": "unreadable", "why": "access is denied", "paths": null,
                }])
            );

            let root = args["root_generation"].as_u64().unwrap();
            let answer = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();
            assert_eq!(answer.written.len(), 3);
            let skipped: Vec<(&str, &str)> = answer
                .skipped
                .iter()
                .map(|skip| (skip.path.as_str(), skip.reason.code.as_str()))
                .collect();
            assert_eq!(skipped, [("dead", LIBRARY_UNAVAILABLE)]);
        }

        // ADR-0400 decisions 2, 7, 12 and 20: the names are judged before anything, and a mount that cannot be read —
        //   absent, or held as overlapping another — is refused `mount-unusable`, nothing applied.
        #[test]
        fn a_rename_refuses_bad_names_and_a_mount_it_cannot_read() {
            let disk = two_mounts();
            let svc = started(disk.start());
            let root = svc.numbers().root;
            let cases = [
                ("engram", "x", RefusalClass::Conflict, MOUNT_READ_ONLY),
                (
                    "team",
                    "engram",
                    RefusalClass::InvalidArgument,
                    MOUNT_NAME_INVALID,
                ),
                (
                    "team",
                    ".x",
                    RefusalClass::InvalidArgument,
                    MOUNT_NAME_INVALID,
                ),
                ("team", "library", RefusalClass::Conflict, MOUNT_NAME_TAKEN),
                ("nope", "x", RefusalClass::NotFound, MOUNT_NOT_FOUND),
            ];
            for (mount, to, class, code) in cases {
                let (got_class, got_code, _) =
                    code_of(svc.rename_mount(mount, to, true, None, None).unwrap_err());
                assert_eq!(
                    (got_class, got_code.as_str()),
                    (class, code),
                    "{mount} → {to}"
                );
            }
            lock(&disk.ops.0).insert(disk.at("lib2"), FolderStatus::Missing);
            let (class, code, args) =
                code_of(svc.rename_mount("team", "x", true, None, None).unwrap_err());
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_UNUSABLE)
            );
            assert_eq!(args["mount"], json!("team"));
            assert_eq!(args["state"], json!("missing"));
            assert_eq!(svc.numbers().root, root, "nothing applied");

            let overlapping = overlapping_disk();
            let svc = overlapping.start();
            assert!(is_held(&mount_row(&svc.status(), "inner").state));
            let (class, code, args) = code_of(
                svc.rename_mount("inner", "x", true, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_UNUSABLE)
            );
            assert!(args["why"].as_str().unwrap().contains("overlaps"), "{args}");
        }

        // ADR-0400 decision 12: a case-only rename is allowed — the references are respelled, and nothing counts as
        //   already pointing at the new spelling.
        #[test]
        fn a_case_only_rename_respells_the_mount_and_its_references() {
            let disk = referencing();
            let svc = started(disk.start());
            let (_, _, args) = code_of(
                svc.rename_mount("team", "Team", false, None, None)
                    .unwrap_err(),
            );
            // Nothing newly points at it: `library/P.yaml`'s `team/a.md` is not counted where keys ignore case.
            assert_eq!(
                args["plan"]["already_pointing"],
                json!({ "presets": 0, "agents": 0 })
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answer = svc
                .rename_mount(
                    "team",
                    "Team",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "Team").as_str()),
                )
                .unwrap();
            assert_eq!(answer.already_pointing, Pointing::default());
            assert_eq!(row_names(&answer.status), ["library", "Team", "engram"]);
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- Team/a.md\n")
            );
            assert_eq!(stored(&disk)[1].0, "Team");
        }

        // ADR-0291 · ADR-0379 · ADR-0400 decision 12: agents to change while the agent store is read-only refuse with
        //   the store's refusal before any file is written.
        #[test]
        fn a_read_only_agent_store_refuses_the_rename_before_any_file_is_written() {
            let disk = referencing();
            let registry = Arc::new(ProfileRegistry::new(Arc::new(NewerAgents(vec![agent(&[
                "team/a.md",
            ])]))));
            let svc = started(disk.start_registry(registry));
            let _port = plugged(&svc);
            let root = svc.numbers().root;
            let refusal = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap_err();
            assert!(
                matches!(
                    refusal,
                    LibraryRefusal::Store(StoreError::Refused(Refusal::Newer { .. }))
                ),
                "{refusal:?}"
            );
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- team/a.md\n")
            );
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            assert_eq!(svc.numbers().root, root);
        }

        // ADR-0400 decision 12 · ADR-0407: a store refusal at the agents step comes after the raise — the answer carries
        //   `stopped` and the new root generation, the old name stays. The repeat is the ordinary call: with the tokens
        //   confirmed before the stop it answers the plan of what is left (no file — all were written — and no
        //   reference left unread, so it needs no confirmation and the supplied digest — no longer this plan's — is
        //   answered `mount-rename-unconfirmed`), and that plan's own tokens finish it.
        #[test]
        fn a_store_refusal_at_the_agents_step_answers_the_raised_generation_and_the_repeat_finishes(
        ) {
            let disk = referencing();
            let svc = started(disk.start_with(ROOT_BASE, vec![agent(&["team/a.md"])]));
            let id = svc.values.registry.list()[0].id;
            let port = plugged(&svc);
            *lock(&port.fail) = Some(ProfileEditError::Store(StoreError::Refused(
                Refusal::Unreadable,
            )));
            let root = svc.numbers().root;
            let confirmed = digest(&svc, "team", "shared");

            let stopped = svc
                .rename_mount("team", "shared", true, Some(root), Some(&confirmed))
                .unwrap();

            let stop = stopped.stopped.as_ref().expect("stopped at the agents");
            assert_eq!(stop.path, None);
            assert!(matches!(stop.refusal, LibraryRefusal::Store(_)), "{stop:?}");
            assert_eq!(stopped.written, ["library/P.yaml", "team/Q.yaml"]);
            assert_eq!(
                stopped.status.root_generation,
                root + 1,
                "raised before the agents"
            );
            assert_eq!(row_names(&stopped.status), ["library", "team", "engram"]);
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);

            // The repeat with the tokens confirmed before the stop: the plan of what is left, nothing applied.
            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&confirmed))
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(
                args["plan"]["files"],
                json!([]),
                "the files written left the plan"
            );
            assert_eq!(args["plan"]["unread"], json!([]));
            assert_eq!(args["plan"]["agents"].as_array().unwrap().len(), 1);
            assert_eq!(args["root_generation"], json!(root + 1));
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);

            // That plan confirmed with its own tokens finishes the rename.
            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    args["root_generation"].as_u64(),
                    args["plan_digest"].as_str(),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert!(done.written.is_empty());
            assert_eq!(row_names(&done.status), ["library", "shared", "engram"]);
            assert_eq!(presets_of(&svc, id), ["shared/a.md"]);
            assert!(has(&svc, "shared/Q.yaml"));
            assert_eq!(stored(&disk)[1].0, "shared", "saved");
        }

        // ADR-0405 decision 2 · ADR-0407: after a stop at the agents step, a plan of what is left that still needs
        //   confirmation — a reference left unread — answers the tokens confirmed before the stop
        //   `root-generation-stale` (the generation moved) with that plan; its own tokens finish the rename.
        #[test]
        fn a_repeat_after_an_agents_step_stop_with_references_left_unread_is_stale_with_the_plan_of_what_is_left(
        ) {
            let (disk, svc) = zed_walked_then_unreadable(true);
            let new = agent(&["team/a.md"]);
            let id = new.id;
            assert!(svc.values.registry.try_upsert_if(new, || true).unwrap());
            let port = plugged(&svc);
            *lock(&port.fail) = Some(ProfileEditError::Store(StoreError::Refused(
                Refusal::Unreadable,
            )));
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            let root = args["root_generation"].as_u64().unwrap();
            let confirmed = args["plan_digest"].as_str().unwrap().to_owned();
            let stopped = svc
                .rename_mount("team", "shared", true, Some(root), Some(&confirmed))
                .unwrap();
            assert!(stopped.stopped.is_some());
            assert_eq!(stopped.written, ["library/P.yaml"]);
            assert_eq!(stopped.status.root_generation, root + 1);

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&confirmed))
                    .unwrap_err(),
            );

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, ROOT_GENERATION_STALE)
            );
            assert_eq!(args["current"], json!(root + 1));
            assert_eq!(args["plan"]["files"], json!([]));
            assert_eq!(args["plan"]["unread"][0]["paths"], json!(["zed/Z.yaml"]));
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);
            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    args["current"].as_u64(),
                    args["plan_digest"].as_str(),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert_eq!(skipped_rows(&done), [("zed/Z.yaml", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );
            assert_eq!(presets_of(&svc, id), ["shared/a.md"]);
            assert_eq!(
                text(&disk, "lib3", "Z.yaml"),
                preset("instructions:\n- team/a.md\n")
            );
        }

        // ADR-0335 · ADR-0400 decision 12 · ADR-0407: a read or write failure stops the walk before the agents and the
        //   registration; the files written point at the new name. A stop before any write answers its own refusal. The
        //   repeat with the tokens confirmed before the stop rewrites nothing and answers the plan of what is left; that
        //   plan, confirmed, finishes the rename and saves it.
        #[test]
        fn a_file_failure_stops_before_the_agents_and_the_registration_and_the_repeat_answers_what_is_left(
        ) {
            let disk = referencing();
            let svc = started(disk.start_with(ROOT_BASE, vec![agent(&["team/a.md"])]));
            let id = svc.values.registry.list()[0].id;
            let port = plugged(&svc);
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            disk.mem("lib").fail_read("P.yaml");
            let (class, code, _) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Unavailable, launch::LIBRARY_READ_FAILED)
            );
            disk.mem("lib").heal_read("P.yaml");
            disk.mem("lib2").fail_read("Q.yaml");

            // The same call: nothing was written, so the confirmed plan still holds.
            let stopped = svc
                .rename_mount("team", "shared", true, Some(root), Some(&answered))
                .unwrap();

            let stop = stopped.stopped.as_ref().expect("stopped at the file");
            assert_eq!(stop.path.as_deref(), Some("team/Q.yaml"));
            assert_eq!(stopped.written, ["library/P.yaml"]);
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- shared/a.md\n")
            );
            assert_eq!(row_names(&stopped.status), ["library", "team", "engram"]);
            assert_eq!(stopped.status.root_generation, root, "not raised");
            assert!(
                lock(&port.rewrites).is_empty(),
                "the agents were not rewritten"
            );
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);

            // The repeat with the tokens confirmed before the stop, once the cause is cleared: nothing rewritten, the
            //   plan of what is left answered.
            disk.mem("lib2").heal_read("Q.yaml");
            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(plan_paths(&args), [json!("team/Q.yaml")]);
            assert_eq!(args["root_generation"], json!(root));
            assert_ne!(
                args["plan_digest"],
                json!(answered),
                "`library/P.yaml` left the plan"
            );
            assert_eq!(
                text(&disk, "lib2", "Q.yaml"),
                preset("instructions:\n- team/a.md\n- library/b.md\n"),
                "nothing rewritten"
            );
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);

            // That plan confirmed with its own tokens: the rename finishes and is saved.
            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    args["root_generation"].as_u64(),
                    args["plan_digest"].as_str(),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert_eq!(done.written, ["shared/Q.yaml"]);
            assert_eq!(row_names(&done.status), ["library", "shared", "engram"]);
            assert_eq!(presets_of(&svc, id), ["shared/a.md"]);
            assert_eq!(done.status.config, ConfigHealth::Ok);
            assert_eq!(stored(&disk)[1].0, "shared", "saved");
        }

        // ADR-0381 · ADR-0400 decision 14: after a failed save the rename is live and `unsaved`; repeating it — judged
        //   before any generation comparison — only saves when no file holds the old name, answers the plan when files
        //   do, and then walks and saves; once saved, the old name is not found.
        #[test]
        fn a_repeated_rename_after_a_failed_save_saves_or_answers_the_plan() {
            let disk = two_mounts();
            let svc = started(disk.start());
            let old_root = svc.numbers().root;
            svc.config.fail_writes(Some("disk full"));
            let (_, code, _) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, LIBRARY_CONFIG_WRITE_FAILED);
            let status = svc.status();
            assert_eq!(row_names(&status), ["library", "shared", "engram"], "live");
            assert!(matches!(status.config, ConfigHealth::Unsaved { .. }));
            let (_, code, _) = code_of(
                svc.rename_mount("teem", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_NOT_FOUND, "a mistyped name is still refused");
            svc.config.fail_writes(None);

            let repeated = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(old_root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();
            assert_eq!(
                repeated.status.config,
                ConfigHealth::Ok,
                "saved, the old generation not compared"
            );
            assert_eq!(repeated.status.root_generation, old_root + 1);
            assert_eq!(stored(&disk)[1].0, "shared");
            let (_, code, _) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_NOT_FOUND, "once saved");

            // With a file still holding the old name.
            svc.config.fail_writes(Some("disk full"));
            let (_, code, _) = code_of(
                svc.rename_mount("shared", "crew", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, LIBRARY_CONFIG_WRITE_FAILED);
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- shared/a.md\n"));
            let _ = svc.refresh(Some(&lp("library")));
            svc.config.fail_writes(None);
            let (_, code, args) = code_of(
                svc.rename_mount("shared", "crew", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            let root = args["root_generation"].as_u64().unwrap();
            assert_eq!(root, svc.numbers().root, "the current generation");
            let walked = svc
                .rename_mount(
                    "shared",
                    "crew",
                    true,
                    Some(root),
                    Some(digest(&svc, "shared", "crew").as_str()),
                )
                .unwrap();
            assert_eq!(walked.written, ["library/P.yaml"]);
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- crew/a.md\n")
            );
            assert_eq!(walked.status.config, ConfigHealth::Ok);
            assert_eq!(stored(&disk)[1].0, "crew");
        }

        // ADR-0400 decisions 12 and 16: edits queued behind a rename's `Root` ticket meet the new name or a stale
        //   generation — none writes `team/…` after the rename.
        #[test]
        fn edits_queued_during_a_rename_meet_the_new_name_or_a_stale_generation() {
            let disk = referencing();
            let svc = started(disk.start_with(ROOT_BASE, vec![agent(&["team/a.md"]), agent(&[])]));
            let ids: Vec<AgentId> = {
                let mut profiles = svc.values.registry.list();
                profiles.sort_by_key(|profile| profile.presets.is_empty());
                profiles.iter().map(|profile| profile.id).collect()
            };
            let (attached, other) = (ids[0], ids[1]);
            let port = plugged(&svc);
            let (entered_tx, entered) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            *lock(&port.entered) = Some(entered_tx);
            *lock(&port.release) = Some(release_rx);
            let root = svc.numbers().root;
            let rename = on_thread(&svc, move |svc| {
                svc.rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename reached its agents pass");
            let blocked = svc.gate.read_blocked();
            let stale_attach = on_thread(&svc, move |svc| {
                svc.attach_preset(other, &["team/a.md".to_owned()], None, Some(root))
            });
            let stale_add = on_thread(&svc, move |svc| {
                let entries = AddEntries::Items(vec![NewItem {
                    path: "team/a.md".to_owned(),
                    backend: None,
                }]);
                svc.preset_add(&lp("library/P.yaml"), &entries, None, Some(root))
            });
            let plain_attach = on_thread(&svc, move |svc| {
                svc.attach_preset(other, &["team/a.md".to_owned()], None, None)
            });
            // All three wait behind the rename's write lock (each reads the library before drawing its ticket).
            wait_read_blocked(&svc, blocked + 3);
            release.send(()).unwrap();
            let renamed = join(rename).unwrap();
            assert_eq!(row_names(&renamed.status), ["library", "shared", "engram"]);
            for (what, refusal) in [
                ("attach", join(stale_attach).err()),
                ("add", join(stale_add).err()),
            ] {
                let (_, code, _) = code_of(refusal.unwrap_or_else(|| panic!("{what} not refused")));
                assert_eq!(code, ROOT_GENERATION_STALE, "{what}");
            }
            let plain = join(plain_attach).unwrap();
            assert_eq!(plain.skipped.len(), 1);
            assert_eq!(
                plain.skipped[0].reason.code, NOT_FOUND,
                "the old name reads absent"
            );
            assert_eq!(presets_of(&svc, attached), ["shared/a.md"]);
            assert!(presets_of(&svc, other).is_empty());
            assert!(!text(&disk, "lib", "P.yaml").contains("team/"));
        }

        /// What `agent.new {presets}` does (agent `verb_new`): judge, insert only while the change number is the one
        /// judged at, judge again otherwise — the registry insert taking the comparison inside its lock.
        fn create(svc: &LibraryService, presets: &[&str]) -> (AgentId, Vec<String>) {
            let presets: Vec<String> = presets.iter().map(|path| (*path).to_owned()).collect();
            loop {
                let judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
                let mut profile = agent(&[]);
                profile.default_preset = judged.attachments.default_preset;
                profile.presets = judged.attachments.presets;
                let at = judged.change.expect("a library preset was named");
                let id = profile.id;
                if svc
                    .values
                    .registry
                    .try_upsert_if(profile, || svc.change_number() == at)
                    .unwrap()
                {
                    let skipped = judged
                        .skipped
                        .into_iter()
                        .map(|skip| format!("{} {}", skip.item, skip.reason.code))
                        .collect();
                    return (id, skipped);
                }
            }
        }

        // ADR-0400 decision 18: an `agent.new` judged before a rename whose insert lands before the agents pass is
        //   rewritten by it; one whose insert lands after the raise finds the number moved, judges again and reports
        //   `team/…` as `not-found` — no agent stores the old name.
        #[test]
        fn an_agent_new_judged_before_a_rename_is_rewritten_by_it_or_judges_again() {
            let disk = two_mounts();
            disk.mem("lib2").file("S.yaml", &preset(""));
            let svc = started(disk.start());
            let port = plugged(&svc);
            let presets = ["team/S.yaml".to_owned()];

            // Inserted before the rename's agents pass: the pass rewrites it.
            let judged = svc.judge_new_presets(&agent(&[]), &presets, None).unwrap();
            let mut early = agent(&[]);
            early.presets = judged.attachments.presets.clone();
            let early_id = early.id;
            let at = judged.change.unwrap();
            assert!(svc
                .values
                .registry
                .try_upsert_if(early, || svc.change_number() == at)
                .unwrap());
            svc.rename_mount("team", "shared", false, None, None)
                .unwrap();
            assert_eq!(presets_of(&svc, early_id), ["shared/S.yaml"]);

            // Judged before the next rename, inserted while its agents pass runs (after the raise): refused, judged
            //   again once the rename is done.
            let judged = svc
                .judge_new_presets(&agent(&[]), &["shared/S.yaml".to_owned()], None)
                .unwrap();
            let at = judged.change.unwrap();
            let (entered_tx, entered) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            *lock(&port.entered) = Some(entered_tx);
            *lock(&port.release) = Some(release_rx);
            let rename = on_thread(&svc, |svc| {
                svc.rename_mount("shared", "crew", false, None, None)
            });
            entered.recv_timeout(Duration::from_secs(10)).unwrap();
            let mut late = agent(&[]);
            late.presets = judged.attachments.presets;
            let late_id = late.id;
            assert!(
                !svc.values
                    .registry
                    .try_upsert_if(late, || svc.change_number() == at)
                    .unwrap(),
                "the raised number refuses the insert"
            );
            release.send(()).unwrap();
            join(rename).unwrap();
            assert!(svc.values.registry.get(late_id).is_none());
            let (id, skipped) = create(&svc, &["shared/S.yaml"]);
            assert_eq!(skipped, [format!("shared/S.yaml {NOT_FOUND}")]);
            assert!(presets_of(&svc, id).is_empty());
            assert_eq!(presets_of(&svc, early_id), ["crew/S.yaml"]);
        }

        /// Waits (at most 10 s) until `n` reads of the library lock in all have found it held for writing.
        fn wait_read_blocked(svc: &LibraryService, n: usize) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while svc.gate.read_blocked() < n {
                assert!(
                    Instant::now() < deadline,
                    "fewer than {n} reads waited on the library lock"
                );
                thread::sleep(Duration::from_millis(2));
            }
        }

        /// An agent store whose `status` parks once armed: it tells the sender, then waits for the receiver — a rename
        /// asks it between its plan and its ticket when the plan names an agent.
        struct ParkedStatus {
            agents: Vec<AgentProfile>,
            door: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        }

        impl ProfileStore for ParkedStatus {
            fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
                Ok(())
            }
            fn load(&self) -> Vec<AgentProfile> {
                self.agents.clone()
            }
            fn status(&self) -> StoreStatus {
                let door = lock(&self.door).take();
                if let Some((entered, release)) = door {
                    entered.send(()).unwrap();
                    release
                        .recv_timeout(Duration::from_secs(20))
                        .expect("the test releases the store status");
                }
                StoreStatus::Writable
            }
        }

        /// Three mounts — [`two_mounts`] plus `zed` at `lib3`.
        fn three_mounts() -> OnDisk {
            let disk = OnDisk::new(&["lib", "lib2", "lib3"], "lib", None);
            std::fs::write(
                disk.config_path(),
                disk.mounts_doc(&[("library", "default"), ("team", "lib2"), ("zed", "lib3")]),
            )
            .unwrap();
            disk.mem("lib").file("b.md", "b");
            disk.mem("lib2").file("a.md", "a");
            disk
        }

        fn skipped_rows(answer: &crate::rename::RenameAnswer) -> Vec<(&str, &str)> {
            answer
                .skipped
                .iter()
                .map(|skip| (skip.path.as_str(), skip.reason.code.as_str()))
                .collect()
        }

        // ADR-0381 · ADR-0400 decision 14: a case-only rename whose save failed, repeated, answers success and saves —
        //   where path keys ignore case the old spelling finds the live mount ("already so named"), elsewhere it is the
        //   repeat; either way library.json ends up with the new spelling.
        #[test]
        fn a_case_only_rename_repeated_after_a_failed_save_saves_it() {
            let disk = two_mounts();
            let svc = started(disk.start());
            svc.config.fail_writes(Some("disk full"));
            let (_, code, _) = code_of(
                svc.rename_mount("team", "Team", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, LIBRARY_CONFIG_WRITE_FAILED);
            let status = svc.status();
            assert_eq!(row_names(&status), ["library", "Team", "engram"], "live");
            assert!(matches!(status.config, ConfigHealth::Unsaved { .. }));
            assert_eq!(stored(&disk)[1].0, "team", "not saved");
            svc.config.fail_writes(None);

            let repeated = svc.rename_mount("team", "Team", false, None, None).unwrap();

            assert_eq!(repeated.status.config, ConfigHealth::Ok, "saved");
            assert_eq!(stored(&disk)[1].0, "Team");
            assert_eq!(row_names(&repeated.status), ["library", "Team", "engram"]);
            assert_eq!(repeated.already_pointing, Pointing::default());
        }

        // ADR-0335 · ADR-0400 decisions 12 and 17: a mount whose folder is lost while the rename's hold walks it (its
        //   file reads not found and the folder no longer stats) stops the walk before the agents and the registration,
        //   and is recorded so: the repeat, the mount still gone, names it unread and asks for confirmation (ADR-0407),
        //   and confirmed it finishes, skipping its file. A file deleted outside Engram from a folder that is there is
        //   skipped as not found.
        #[test]
        fn a_mount_lost_during_the_hold_stops_the_rename_and_a_file_deleted_outside_is_skipped() {
            let disk = three_mounts();
            for (folder, rel) in [("lib", "P.yaml"), ("lib2", "Q.yaml"), ("lib3", "Z.yaml")] {
                disk.mem(folder)
                    .file(rel, &preset("instructions:\n- team/a.md\n"));
            }
            let svc = started(disk.start());
            let root = svc.numbers().root;
            let confirmed = digest(&svc, "team", "shared");
            let (entered, release) = disk.store.door_read(disk.at("lib").join("P.yaml"));
            let tokens = confirmed.clone();
            let rename = on_thread(&svc, move |svc| {
                svc.rename_mount("team", "shared", true, Some(root), Some(&tokens))
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename's walk reads library/P.yaml");
            // Lost inside the hold, after every stat the call took before it.
            disk.mem("lib3").remove("Z.yaml");
            lock(&disk.ops.0).insert(disk.at("lib3"), FolderStatus::Missing);
            release.send(()).unwrap();

            let stopped = join(rename).unwrap();

            assert_eq!(stopped.written, ["library/P.yaml", "team/Q.yaml"]);
            assert_eq!(
                row_names(&stopped.status),
                ["library", "team", "zed", "engram"]
            );
            assert_eq!(stopped.status.root_generation, root, "not raised");
            let stop = stopped.stopped.expect("stopped at the lost mount");
            assert_eq!(stop.path.as_deref(), Some("zed/Z.yaml"));
            let (class, code, args) = code_of(stop.refusal);
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
            );
            assert_eq!(args["mount"], json!("zed"));
            let zed = MountName::parse("zed").unwrap();
            assert!(
                matches!(
                    svc.with_read(|state| state.seen(&zed)).state,
                    MountState::Missing { .. }
                ),
                "recorded gone"
            );

            // ADR-0405 decisions 2 and 3 · ADR-0407: the repeat with the tokens confirmed before the stop does not
            //   finish silently — its plan names the lost mount under `unread` and asks for confirmation.
            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&confirmed))
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(args["plan"]["files"], json!([]));
            let unread: Vec<serde_json::Value> = args["plan"]["unread"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| json!([mount["mount"], mount["state"], mount["paths"]]))
                .collect();
            assert_eq!(unread, [json!(["zed", "missing", ["zed/Z.yaml"]])]);
            assert_eq!(
                row_names(&svc.status()),
                ["library", "team", "zed", "engram"]
            );

            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    args["root_generation"].as_u64(),
                    args["plan_digest"].as_str(),
                )
                .unwrap();

            assert!(done.stopped.is_none());
            assert_eq!(skipped_rows(&done), [("zed/Z.yaml", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );

            // Deleted outside Engram, its folder there: skipped as not found, the rename finishes.
            let disk = three_mounts();
            for (folder, rel) in [("lib", "P.yaml"), ("lib3", "Z.yaml")] {
                disk.mem(folder)
                    .file(rel, &preset("instructions:\n- team/a.md\n"));
            }
            let svc = started(disk.start());
            disk.mem("lib3").remove("Z.yaml");
            let root = svc.numbers().root;
            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert_eq!(done.written, ["library/P.yaml"]);
            assert_eq!(skipped_rows(&done), [("zed/Z.yaml", NOT_FOUND)]);
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );
        }

        // ADR-0400 decisions 12 and 17: a mount recorded readable whose folder is gone since its last walk (no scan
        //   since) is stat'ed before the plan and recorded so — the plan names it under `unread`, leaves its files out,
        //   and the confirmed call finishes, skipping them `library-unavailable`.
        #[test]
        fn a_mount_lost_before_the_rename_is_named_unread_and_its_files_are_skipped() {
            let disk = three_mounts();
            for (folder, rel) in [("lib", "P.yaml"), ("lib2", "Q.yaml"), ("lib3", "Z.yaml")] {
                disk.mem(folder)
                    .file(rel, &preset("instructions:\n- team/a.md\n"));
            }
            let svc = started(disk.start());
            disk.mem("lib3").remove("Z.yaml");
            lock(&disk.ops.0).insert(disk.at("lib3"), FolderStatus::Missing);

            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );

            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            let files: Vec<serde_json::Value> = args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| json!([mount["mount"], mount["paths"]]))
                .collect();
            assert_eq!(
                files,
                [
                    json!(["library", ["library/P.yaml"]]),
                    json!(["team", ["team/Q.yaml"]]),
                ]
            );
            let unread: Vec<(&str, &str)> = args["plan"]["unread"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| {
                    (
                        mount["mount"].as_str().unwrap(),
                        mount["state"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(unread, [("zed", "missing")]);
            let root = args["root_generation"].as_u64().unwrap();
            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert_eq!(done.written, ["library/P.yaml", "shared/Q.yaml"]);
            assert_eq!(skipped_rows(&done), [("zed/Z.yaml", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );
        }

        // ADR-0335 · ADR-0400 decision 12: the renamed mount itself lost while the hold walks it stops the rename with
        //   `library-unavailable` naming it, nothing written and the old name registered; the repeat, the mount still
        //   gone, is refused `mount-unusable` before anything.
        #[test]
        fn the_renamed_mount_lost_during_the_hold_stops_the_rename_and_its_repeat_is_refused_unusable(
        ) {
            let disk = two_mounts();
            disk.mem("lib2")
                .file("Q.yaml", &preset("instructions:\n- team/a.md\n"));
            let svc = started(disk.start());
            let root = svc.numbers().root;
            let (entered, release) = disk.store.door_read(disk.at("lib2").join("Q.yaml"));
            let rename = on_thread(&svc, move |svc| {
                svc.rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename's walk reads team/Q.yaml");
            disk.mem("lib2").remove("Q.yaml");
            lock(&disk.ops.0).insert(disk.at("lib2"), FolderStatus::Missing);
            release.send(()).unwrap();

            let (class, code, args) = code_of(join(rename).unwrap_err());

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Unavailable, LIBRARY_UNAVAILABLE)
            );
            assert_eq!(args["mount"], json!("team"));
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            assert_eq!(svc.numbers().root, root, "nothing raised");
            let (class, code, args) = code_of(
                svc.rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_UNUSABLE)
            );
            assert_eq!(args["state"], json!("missing"));
        }

        /// [`three_mounts`] with `zed/Z.yaml` listing `team/a.md` (and `library/P.yaml` too when `also_readable`),
        /// walked, and then `zed` unreadable.
        fn zed_walked_then_unreadable(also_readable: bool) -> (OnDisk, Arc<LibraryService>) {
            let disk = three_mounts();
            disk.mem("lib3")
                .file("Z.yaml", &preset("instructions:\n- team/a.md\n"));
            if also_readable {
                disk.mem("lib")
                    .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            }
            let svc = started(disk.start());
            lock(&disk.ops.0).insert(
                disk.at("lib3"),
                FolderStatus::Unreadable("access is denied".to_owned()),
            );
            let _ = svc.refresh(Some(&lp("zed")));
            let zed = MountName::parse("zed").unwrap();
            let seen = svc.with_read(|state| state.seen(&zed));
            assert!(seen.walked && !seen.usable(), "{seen:?}");
            (disk, svc)
        }

        // ADR-0405 decision 2: a mount walked earlier that cannot be read now is named
        //   under the plan's `unread` with its referencing files, left out of `files`. Files left so ask the same
        //   confirmation as files to rewrite, even with nothing readable to rewrite; confirmed, the rename skips them
        //   with `library-unavailable`, untouched.
        #[test]
        fn a_file_in_a_mount_that_cannot_be_read_is_left_out_of_the_plan_and_skipped() {
            // Only the unreadable mount's file refers to `team`: asked first, nothing applied.
            let (disk, svc) = zed_walked_then_unreadable(false);
            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(args["plan"]["files"], json!([]));
            assert_eq!(
                args["plan"]["unread"],
                json!([{
                    "mount": "zed", "state": "unreadable", "why": "access is denied",
                    "paths": ["zed/Z.yaml"],
                }])
            );
            assert_eq!(
                row_names(&svc.status()),
                ["library", "team", "zed", "engram"]
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            let answer = svc
                .rename_mount("team", "shared", true, Some(root), Some(&answered))
                .unwrap();
            assert!(answer.stopped.is_none() && answer.written.is_empty());
            assert_eq!(skipped_rows(&answer), [("zed/Z.yaml", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                row_names(&answer.status),
                ["library", "shared", "zed", "engram"]
            );
            assert_eq!(
                text(&disk, "lib3", "Z.yaml"),
                preset("instructions:\n- team/a.md\n")
            );

            // With a readable file too: the plan lists only it and names `zed` under `unread`.
            let (disk, svc) = zed_walked_then_unreadable(true);
            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            let files: Vec<serde_json::Value> = args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| json!([mount["mount"], mount["paths"]]))
                .collect();
            assert_eq!(files, [json!(["library", ["library/P.yaml"]])]);
            assert_eq!(
                args["plan"]["unread"],
                json!([{
                    "mount": "zed", "state": "unreadable", "why": "access is denied",
                    "paths": ["zed/Z.yaml"],
                }])
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answer = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
                .unwrap();
            assert_eq!(answer.written, ["library/P.yaml"]);
            assert_eq!(skipped_rows(&answer), [("zed/Z.yaml", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                text(&disk, "lib3", "Z.yaml"),
                preset("instructions:\n- team/a.md\n")
            );
        }

        // ADR-0400 decision 12 (decisions A and B): the digest pins the unread side too — a mount that comes back
        //   between the answered plan and the confirming call moves its file to the side rewritten, so the call
        //   applies nothing and answers the fresh plan, whose tokens then rewrite it; a never-walked mount that comes
        //   back holding no reference changes the plan as well.
        #[test]
        fn a_mount_that_comes_back_after_the_plan_is_asked_again() {
            let (disk, svc) = zed_walked_then_unreadable(false);
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            lock(&disk.ops.0).remove(&disk.at("lib3"));

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(plan_paths(&args), [json!("zed/Z.yaml")]);
            assert_eq!(args["plan"]["unread"], json!([]));
            let fresh = args["plan_digest"].as_str().unwrap().to_owned();
            assert_ne!(fresh, answered);
            assert!(text(&disk, "lib3", "Z.yaml").contains("- team/a.md"));
            let done = svc
                .rename_mount("team", "shared", true, Some(root), Some(&fresh))
                .unwrap();
            assert_eq!(done.written, ["zed/Z.yaml"]);

            // A mount never walked (its references unknown) that comes back holding none: the plan the caller
            //   confirmed is no longer the one the call would carry out.
            let disk = three_mounts();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            lock(&disk.ops.0).insert(
                disk.at("lib3"),
                FolderStatus::Unreadable("access is denied".to_owned()),
            );
            let svc = started(disk.start());
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(args["plan"]["unread"][0]["paths"], json!(null));
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            lock(&disk.ops.0).remove(&disk.at("lib3"));
            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            assert_eq!(plan_paths(&args), [json!("library/P.yaml")]);
            assert_eq!(args["plan"]["unread"], json!([]));
            assert!(text(&disk, "lib", "P.yaml").contains("- team/a.md"));
        }

        // ADR-0405 decision 2: a mount never walked this run leaves unknown references — it alone asks
        //   the confirmation; a rename with nothing to rewrite and nothing left unread — an unreadable mount holding no
        //   reference to the renamed one — runs without `yes`.
        #[test]
        fn a_mount_never_walked_asks_first_and_nothing_left_unread_asks_nothing() {
            let disk = three_mounts();
            lock(&disk.ops.0).insert(
                disk.at("lib3"),
                FolderStatus::Unreadable("access is denied".to_owned()),
            );
            let svc = started(disk.start());
            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            assert_eq!(args["plan"]["unread"][0]["mount"], json!("zed"));
            assert_eq!(args["plan"]["unread"][0]["paths"], json!(null), "unknown");
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            let done = svc
                .rename_mount("team", "shared", true, Some(root), Some(&answered))
                .unwrap();
            assert_eq!(skipped_rows(&done), [("zed", LIBRARY_UNAVAILABLE)]);
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );

            // Walked, then unreadable, holding no reference to `team`: nothing is left unread.
            let disk = three_mounts();
            disk.mem("lib3")
                .file("Z.yaml", &preset("instructions:\n- library/b.md\n"));
            let svc = started(disk.start());
            lock(&disk.ops.0).insert(
                disk.at("lib3"),
                FolderStatus::Unreadable("access is denied".to_owned()),
            );
            let _ = svc.refresh(Some(&lp("zed")));
            let done = svc
                .rename_mount("team", "shared", false, None, None)
                .unwrap();
            assert!(done.skipped.is_empty() && done.written.is_empty());
            assert_eq!(
                row_names(&done.status),
                ["library", "shared", "zed", "engram"]
            );
        }

        // ADR-0400 decision 14: the repeat needs the failed save to have been this rename — the last saved list names
        //   the old name at the live new name's folder. A failed removal of `team`, or a failed rename of `team` to a
        //   third name, then a rename of `team` to another mount answers `mount-not-found`, nothing rewritten or saved.
        #[test]
        fn a_failed_save_that_was_not_this_rename_does_not_make_it_a_repeat() {
            // A failed removal, then a rename onto another mount that would re-point `team/…` at `zed/…`.
            let disk = three_mounts();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            let svc = started(disk.start());
            svc.config.fail_writes(Some("disk full"));
            let (_, code, _) = code_of(svc.remove_mount("team").err().expect("the save fails"));
            assert_eq!(code, LIBRARY_CONFIG_WRITE_FAILED);
            svc.config.fail_writes(None);
            let root = svc.numbers().root;
            for (yes, given) in [(false, None), (true, Some(root))] {
                let (class, code, _) = code_of(
                    svc.rename_mount("team", "zed", yes, given, None)
                        .unwrap_err(),
                );
                assert_eq!(
                    (class, code.as_str()),
                    (RefusalClass::NotFound, MOUNT_NOT_FOUND)
                );
            }
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- team/a.md\n")
            );
            assert!(matches!(svc.status().config, ConfigHealth::Unsaved { .. }));

            // A failed rename to `shared`, then a rename of `team` to `zed`.
            let disk = three_mounts();
            let svc = started(disk.start());
            svc.config.fail_writes(Some("disk full"));
            let (_, code, _) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, LIBRARY_CONFIG_WRITE_FAILED);
            svc.config.fail_writes(None);
            let (class, code, _) = code_of(
                svc.rename_mount("team", "zed", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::NotFound, MOUNT_NOT_FOUND)
            );
            let status = svc.status();
            assert!(
                matches!(status.config, ConfigHealth::Unsaved { .. }),
                "not saved"
            );
            assert_eq!(stored(&disk)[1].0, "team");
        }

        // ADR-0400 decision 12: the confirmation is judged again inside the hold — a plan that rewrote no file, then
        //   a change ahead of the rename's ticket that makes a preset refer to the mount, answers
        //   `mount-rename-unconfirmed` with a fresh plan naming that file; nothing is rewritten or renamed.
        #[test]
        fn a_file_that_comes_to_refer_to_the_mount_after_a_plan_with_none_asks_again() {
            let disk = two_mounts();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- library/b.md\n"));
            let store = Arc::new(ParkedStatus {
                agents: vec![agent(&["team/a.md"])],
                door: Mutex::new(None),
            });
            let registry = Arc::new(ProfileRegistry::new(
                Arc::clone(&store) as Arc<dyn ProfileStore>
            ));
            let svc = started(disk.start_registry(registry));
            let id = svc.values.registry.list()[0].id;
            let _port = plugged(&svc);
            let root = svc.numbers().root;
            let (entered_tx, entered) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            *lock(&store.door) = Some((entered_tx, release_rx));
            let rename = on_thread(&svc, |svc| {
                svc.rename_mount("team", "shared", false, None, None)
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename built its plan and asked the agent store");
            // Ahead of the rename's ticket: `library/P.yaml` comes to list `team/a.md`.
            let entries = AddEntries::Items(vec![NewItem {
                path: "team/a.md".to_owned(),
                backend: None,
            }]);
            assert!(svc
                .preset_add(&lp("library/P.yaml"), &entries, None, None)
                .is_ok());
            release.send(()).unwrap();

            let (class, code, args) = code_of(join(rename).unwrap_err());

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            let paths: Vec<serde_json::Value> = args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|mount| mount["paths"].clone())
                .collect();
            assert_eq!(paths, [json!(["library/P.yaml"])], "a fresh plan");
            assert_eq!(args["root_generation"], json!(root));
            assert_eq!(svc.numbers().root, root, "nothing raised");
            assert!(text(&disk, "lib", "P.yaml").contains("- team/a.md"));
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);
        }

        // ADR-0400 decision 18: the agents pass reads the registry after the raise — an `agent.new` judged before the
        //   rename whose insert lands while the hold walks the preset files (before the raise) is rewritten by it.
        #[test]
        fn an_agent_new_inserted_while_the_walk_runs_is_rewritten_by_the_agents_pass() {
            let disk = two_mounts();
            disk.mem("lib2").file("S.yaml", &preset(""));
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            let svc = started(disk.start());
            let _port = plugged(&svc);
            let judged = svc
                .judge_new_presets(&agent(&[]), &["team/S.yaml".to_owned()], None)
                .unwrap();
            let at = judged.change.expect("a library preset was named");
            let mut new = agent(&[]);
            new.presets = judged.attachments.presets;
            let id = new.id;
            let root = svc.numbers().root;
            let (entered, release) = disk.store.door_read(disk.at("lib").join("P.yaml"));
            let rename = on_thread(&svc, move |svc| {
                svc.rename_mount(
                    "team",
                    "shared",
                    true,
                    Some(root),
                    Some(digest(&svc, "team", "shared").as_str()),
                )
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename's walk reads library/P.yaml");
            assert!(
                svc.values
                    .registry
                    .try_upsert_if(new, || svc.change_number() == at)
                    .unwrap(),
                "inserted before the raise"
            );
            release.send(()).unwrap();

            let renamed = join(rename).unwrap();

            assert!(renamed.stopped.is_none());
            assert_eq!(presets_of(&svc, id), ["shared/S.yaml"]);
            assert!(renamed
                .fixed
                .contains(&("team/S.yaml".to_owned(), "shared/S.yaml".to_owned())));
        }

        /// [`two_mounts`] with `library/P.yaml` listing `team/a.md` and `library/R.yaml` listing only `library/b.md`.
        fn one_referencing() -> OnDisk {
            let disk = two_mounts();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- team/a.md\n"));
            disk.mem("lib")
                .file("R.yaml", &preset("instructions:\n- library/b.md\n"));
            disk
        }

        /// `library/R.yaml` comes to list `team/a.md`, through the edit pipeline.
        fn r_refers_to_team(svc: &LibraryService) {
            let entries = AddEntries::Items(vec![NewItem {
                path: "team/a.md".to_owned(),
                backend: None,
            }]);
            assert!(svc
                .preset_add(&lp("library/R.yaml"), &entries, None, None)
                .is_ok());
        }

        fn plan_paths(args: &serde_json::Value) -> Vec<serde_json::Value> {
            args["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|mount| mount["paths"].as_array().unwrap().clone())
                .collect()
        }

        // ADR-0405 decision 1: `yes` pins the files the caller saw — without the
        //   answered `plan_digest`, or with another one, nothing is applied and the plan is answered again; with it
        //   the rename runs.
        #[test]
        fn a_confirmation_needs_the_plans_digest() {
            let disk = one_referencing();
            let svc = started(disk.start());
            let (_, code, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(code, MOUNT_RENAME_UNCONFIRMED);
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            assert_eq!(answered, digest(&svc, "team", "shared"));

            for given in [None, Some("not the plan's")] {
                let (class, code, args) = code_of(
                    svc.rename_mount("team", "shared", true, Some(root), given)
                        .unwrap_err(),
                );
                assert_eq!(
                    (class, code.as_str()),
                    (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED),
                    "{given:?}"
                );
                assert_eq!(args["plan_digest"], json!(answered), "the plan again");
            }
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- team/a.md\n"),
                "nothing applied"
            );
            assert_eq!(svc.numbers().root, root);

            let done = svc
                .rename_mount("team", "shared", true, Some(root), Some(&answered))
                .unwrap();
            assert_eq!(done.written, ["library/P.yaml"]);
            assert_eq!(row_names(&done.status), ["library", "shared", "engram"]);
        }

        // ADR-0405 decision 1: a file that comes to refer to the mount between the answered plan and
        //   the confirming call is not covered by that confirmation — the call applies nothing and answers the fresh
        //   plan naming it, whose digest then runs the rename over both files.
        #[test]
        fn a_file_that_comes_to_refer_to_the_mount_after_the_answered_plan_is_asked_again() {
            let disk = one_referencing();
            let svc = started(disk.start());
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(plan_paths(&args), [json!("library/P.yaml")]);
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            r_refers_to_team(&svc);

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(
                plan_paths(&args),
                [json!("library/P.yaml"), json!("library/R.yaml")]
            );
            let fresh = args["plan_digest"].as_str().unwrap().to_owned();
            assert_ne!(fresh, answered);
            assert!(text(&disk, "lib", "P.yaml").contains("- team/a.md"));
            assert!(text(&disk, "lib", "R.yaml").contains("- team/a.md"));
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);

            let done = svc
                .rename_mount("team", "shared", true, Some(root), Some(&fresh))
                .unwrap();
            assert_eq!(done.written, ["library/P.yaml", "library/R.yaml"]);
        }

        // ADR-0405 decision 1: the same inside the hold window — a confirmed call whose plan had a file,
        //   then a change ahead of its ticket that makes another preset refer to the mount: the hold's files are not
        //   the confirmed ones, so nothing is rewritten or raised and the fresh plan names both.
        #[test]
        fn a_file_that_comes_to_refer_to_the_mount_before_the_hold_is_asked_again() {
            let disk = one_referencing();
            let store = Arc::new(ParkedStatus {
                agents: vec![agent(&["team/a.md"])],
                door: Mutex::new(None),
            });
            let registry = Arc::new(ProfileRegistry::new(
                Arc::clone(&store) as Arc<dyn ProfileStore>
            ));
            let svc = started(disk.start_registry(registry));
            let id = svc.values.registry.list()[0].id;
            let _port = plugged(&svc);
            let root = svc.numbers().root;
            let answered = digest(&svc, "team", "shared");
            let (entered_tx, entered) = mpsc::channel();
            let (release, release_rx) = mpsc::channel();
            *lock(&store.door) = Some((entered_tx, release_rx));
            let rename = on_thread(&svc, move |svc| {
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
            });
            entered
                .recv_timeout(Duration::from_secs(10))
                .expect("the rename built its plan and asked the agent store");
            r_refers_to_team(&svc);
            release.send(()).unwrap();

            let (class, code, args) = code_of(join(rename).unwrap_err());

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(
                plan_paths(&args),
                [json!("library/P.yaml"), json!("library/R.yaml")],
                "a fresh plan"
            );
            assert_eq!(svc.numbers().root, root, "nothing raised");
            assert_eq!(
                text(&disk, "lib", "P.yaml"),
                preset("instructions:\n- team/a.md\n"),
                "nothing rewritten"
            );
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            assert_eq!(presets_of(&svc, id), ["team/a.md"]);
        }

        // ADR-0405 decision 1: a supplied `plan_digest` is compared even when the plan now needs no
        //   confirmation — the last referencing file stopped referring after the answered plan, so the confirming call
        //   applies nothing and answers the fresh, empty plan; the next call, with no tokens, runs.
        #[test]
        fn a_confirmation_of_a_plan_that_became_empty_is_answered_again() {
            let disk = one_referencing();
            let svc = started(disk.start());
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            let root = args["root_generation"].as_u64().unwrap();
            let answered = args["plan_digest"].as_str().unwrap().to_owned();
            disk.mem("lib")
                .file("P.yaml", &preset("instructions:\n- library/b.md\n"));
            let _ = svc.refresh(Some(&lp("library")));

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, Some(root), Some(&answered))
                    .unwrap_err(),
            );

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(args["plan"]["files"], json!([]));
            assert_eq!(args["plan"]["unread"], json!([]));
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);
            let done = svc
                .rename_mount("team", "shared", false, None, None)
                .unwrap();
            assert_eq!(row_names(&done.status), ["library", "shared", "engram"]);
        }

        // ADR-0405 decision 1 · ADR-0407 (review round 5): a preset that comes to refer to the mount after a stop — here
        //   right after the stopped call's hold is released, before it answers — is never rewritten under a
        //   confirmation the caller holds: the repeat with those tokens answers the plan of what is left naming it, and
        //   only that plan's own tokens rewrite it. Read at the wire, so a stopped answer carrying a token again would
        //   be the one the caller repeats with.
        #[test]
        fn a_file_that_comes_to_refer_to_the_mount_after_a_stop_is_never_rewritten_under_the_old_confirmation(
        ) {
            use engram_dashboard_agent::library_commands::LibraryCommandHost;

            let disk = referencing();
            disk.mem("lib")
                .file("R.yaml", &preset("instructions:\n- library/b.md\n"));
            let svc = started(disk.start());
            let (_, _, args) = code_of(
                svc.rename_mount("team", "shared", false, None, None)
                    .unwrap_err(),
            );
            assert_eq!(
                plan_paths(&args),
                [json!("library/P.yaml"), json!("team/Q.yaml")]
            );
            let root = args["root_generation"].as_u64().unwrap();
            let confirmed = args["plan_digest"].as_str().unwrap().to_owned();
            disk.mem("lib2").fail_read("Q.yaml");
            let editor = Arc::clone(&svc);
            crate::rename::AFTER_HOLD.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || r_refers_to_team(&editor)));
            });

            let stopped = LibraryCommandHost::rename_mount(
                &*svc,
                "team",
                "shared",
                true,
                Some(root),
                Some(&confirmed),
            )
            .map_err(|refusal| format!("{refusal:?}"))
            .unwrap();

            let stopped = serde_json::to_value(stopped).unwrap();
            assert_eq!(stopped["stopped"]["path"], json!("team/Q.yaml"));
            assert_eq!(stopped["written"], json!(["library/P.yaml"]));
            assert!(
                text(&disk, "lib", "R.yaml").contains("- team/a.md"),
                "the edit landed after the hold"
            );
            disk.mem("lib2").heal_read("Q.yaml");
            // The tokens the caller holds after the stop: the stopped answer's, should it carry any, else the ones it
            //   confirmed (a guard against the round-4 token coming back — it reads that field by its old name only).
            let held_generation = stopped["root_generation"].as_u64();
            let held_digest = stopped
                .get("plan_digest")
                .and_then(|digest| digest.as_str())
                .unwrap_or(&confirmed)
                .to_owned();

            let (class, code, args) = code_of(
                svc.rename_mount("team", "shared", true, held_generation, Some(&held_digest))
                    .unwrap_err(),
            );

            assert_eq!(
                (class, code.as_str()),
                (RefusalClass::Conflict, MOUNT_RENAME_UNCONFIRMED)
            );
            assert_eq!(
                plan_paths(&args),
                [json!("library/R.yaml"), json!("team/Q.yaml")],
                "the plan of what is left names the new file"
            );
            assert!(
                text(&disk, "lib", "R.yaml").contains("- team/a.md"),
                "not rewritten"
            );
            assert!(
                text(&disk, "lib2", "Q.yaml").contains("- team/a.md"),
                "not rewritten"
            );
            assert_eq!(row_names(&svc.status()), ["library", "team", "engram"]);

            let done = svc
                .rename_mount(
                    "team",
                    "shared",
                    true,
                    args["root_generation"].as_u64(),
                    args["plan_digest"].as_str(),
                )
                .unwrap();
            assert!(done.stopped.is_none());
            assert_eq!(done.written, ["library/R.yaml", "shared/Q.yaml"]);
            assert_eq!(row_names(&done.status), ["library", "shared", "engram"]);
        }
    }
}
