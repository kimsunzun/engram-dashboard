//! Resolution — 「what it actually gets」: what an agent (or one preset on its own) gets from its attached presets, its
//! problem list and its notices ([`resolve`]).
//!
//! ## Invariants
//!
//! - **Pure** — no disk, no lock, no clock. Items come through [`Lookup`] (the catalog plus the built-ins, or a launch's
//!   overlay); folder entries are only collected here, and their disk status arrives later through
//!   [`Resolution::attach_folder_status`] (phase 2 — the caller stats [`Resolution::folder_paths`] outside every lock).
//! - **Expansion** — for an agent, `engram/default` (when attached) and then the attached presets in order; one preset =
//!   its nested `presets` first, depth first (D1), then its own instructions · settings · skills, each list's common
//!   items followed by the items tagged for the agent's backend (per preset — G51). An item reached a second time stays
//!   at its first place; a preset reached again while it is being expanded is a cycle.
//! - **Backend knowledge comes only from port answers** (TRD S25 step-7 §1-1): what a mode loads (`loads`), which skill
//!   names and formats it refuses (`accepts_skill_name` · `skill_format`), which folder paths its command line cannot
//!   carry (`folder_entry_blocks`) and its notice codes (`notice_codes`). Tags and raw-field names match a backend
//!   through [`Backends::for_word`]; nothing here names a backend.
//! - **Skipped silently** (D2): an item tagged for another backend and a settings field of another backend are neither
//!   loaded nor told about.
//! - The problem rows are the launch block rows (`agent::launch::BlockReason`): the problem list, the tree ⛔ and the
//!   launch block come from the same rows. The split is by row kind: a launch leaves out every ① row (absent) and the
//!   ④ row of an absent mount ([`Resolution::skipped`]) and refuses on every other row ([`Resolution::blocked`]).
//! - **Per mount** (ADR-0400 decision 19): a library item is looked up only when its mount is usable
//!   ([`MountStates`]). A mount that is not registered or not there is absent — its items are skipped; one that exists
//!   but cannot be used stands one ④ row naming it for every item reached in it, and blocks. Other mounts expand
//!   normally; a built-in never depends on a mount.
// ADR-0315
// ADR-0341
// ADR-0359
// ADR-0387
// ADR-0400
// TRD S25 step-7 §3-7

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use engram_dashboard_agent::backend::settings_check::{InstructionLoad, ModeLoads, SkillFormat};
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::{
    escape_controls, BlockReason, LaunchBlocked, Notice, Reason, BUILTIN_UNAVAILABLE,
    FOLDER_ITEM_PREFIX,
};
use engram_dashboard_agent::profile::AgentOutputFormat;
use engram_dashboard_platform::fs::{path_key, PathKey};
use serde_json::{json, Value};

use crate::backends::Backends;
use crate::builtin::Builtins;
use crate::catalog::{key, Catalog, ItemContent, ItemKind};
use crate::folder::FolderStatus;
use crate::format::read::{ITEM_PATH_INVALID, WRONG_KIND};
use crate::format::{conflict_key, ListName, PresetDoc};
use crate::gate::NOT_FOUND;
use crate::mounts::{MountName, Mounts};
use crate::path::{AssetRef, BuiltinId, FolderRef, LibPath, PathError, PresetItem};
use crate::validate::check_skill;
use crate::LIBRARY_UNAVAILABLE;

/// The code of a preset reached again while it is being expanded (`args` = `{path}`).
// TRD S25 step-7 B8
pub const PRESET_CYCLE: &str = "preset-cycle";

/// The code of a built-in listed in a library preset's `presets` (`args` = `{path}`) — a built-in preset can be
/// attached to an agent, not nested in a library preset.
// TRD S25 step-7 B8
pub const BUILTIN_IN_LIBRARY_PRESET: &str = "builtin-in-library-preset";

/// The code of a loaded skill whose name the agent's backend refuses (`args` = `{name}`).
// TRD S25 step-7 B8
pub const SKILL_NAME_INVALID: &str = "skill-name-invalid";

/// The code of a conflict: loaded skills with one collision key (`args` = `{name}` — the key).
// TRD S25 step-7 B8
pub const SKILL_NAME_CONFLICT: &str = "skill-name-conflict";

/// The code of a folder entry whose folder is not there — missing or not a folder (`args` = `{path}`). A ① row: a
/// launch leaves the folder out.
// ADR-0341
// ADR-0387
// TRD S25 step-7 B8
pub const FOLDER_NOT_FOUND: &str = "folder-not-found";

/// The code of a folder entry whose folder exists but cannot be read or did not answer (`args` = `{path, why}` —
/// `why` = the OS error, [`crate::folder::TIMEOUT`], [`crate::folder::NOT_CHECKED`] (never stat'ed — the budget was
/// spent), [`crate::folder::SHARE_NOT_REACHABLE`], or a failed worker ("spawn failed", "worker panicked")). A ② row:
/// it blocks a launch. Its user-facing wording names no cause; `why` is for the log and an LLM caller.
// ADR-0387
pub const FOLDER_UNREADABLE: &str = "folder-unreadable";

/// The code of a folder entry the agent's command line cannot carry (`args` = `{path}`).
// TRD S25 step-7 B8
pub const FOLDER_PATH_CLI_UNSAFE: &str = "folder-path-cli-unsafe";

/// Why a mount a reference names is absent when no mount of that name is registered.
// ADR-0400 decision 19
pub const NOT_REGISTERED: &str = "not registered";

/// One problem row — the row a launch refuses on or leaves out, so the problem list and the launch block come from the
/// same rows. `item` = a
/// library path, `engram/…` or `folder:<normalized path>` (the launch check adds `command-line:<backend word>` and
/// `working-folder:<path>` rows — `agent::launch::BlockReason`); `via` = the presets from the expansion start (for an
/// agent, the attached preset) down to the one that lists the item — empty for an attached preset itself. A conflict
/// carries every skill involved with its own chain. `reason.line` is a line of the referencing preset (`via.last()`; a row
/// for an attached preset itself has `None`) for a row about a reference — `not-found`, `wrong-kind`, `preset-cycle`,
/// `builtin-in-library-preset`, `path-through-link`, a folder entry — and a line of the item's own file for a row about
/// its content when the content gives one (line problems, `yaml-syntax`, settings refusals, skill format). Rows
/// with no line: skill-name checks and conflicts, block ④ (`args` = `{mount, message}`, one per mount), an unparsed
/// attached path.
// The TRD's `Problem { kind, item, reason, via }` cannot carry a conflict's several items (parent §1-3 5).
pub type Problem = BlockReason;

/// What [`resolve`] resolves.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The agent's backend word; `None` = one preset on its own (no agent — everything is shown).
    pub backend: Option<String>,
    /// The agent's mode; `None` with `backend: None`.
    pub mode: Option<AgentOutputFormat>,
    /// Whether `engram/default` is attached (it expands first).
    pub default_preset: bool,
    /// The attached presets, in attach order.
    pub presets: Vec<AssetRef>,
    /// Attached preset spellings that are not references, with why — each is a ② row (an attached part is
    /// required — TRD S25 §1-3 8).
    pub unparsed: Vec<(String, PathError)>,
    /// The state of each mount — decides the rows of the library items reached in a mount that cannot be used.
    pub mounts: MountStates,
}

/// A library mount's state as a computation found it — one rule for every producer ([`RootState::of`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootState {
    /// Not registered ([`NOT_REGISTERED`]), or registered but not there — why: `missing` or `not a folder`. A launch
    /// skips its items.
    Absent(String),
    /// Registered and there but cannot be used — why: the stat's diagnostic, or why its items cannot be read. A launch
    /// refuses (block ④).
    Unreadable(String),
    Ok,
}

impl RootState {
    /// The state of a registered mount from the first-scan entrance's answer (`joined`; a launch passes `Ok`) and the
    /// mount folder's stat (`None` = not taken — the folder decides nothing): absent when the stat says missing or not a
    /// folder; unreadable when the stat says so, or — the folder being there — when the first scan failed; else `Ok`.
    // ADR-0387
    // ADR-0400 decision 19
    pub(crate) fn of(joined: &Result<(), String>, stat: Option<&FolderStatus>) -> RootState {
        match (stat, joined) {
            (Some(status @ (FolderStatus::Missing | FolderStatus::NotADirectory)), _) => {
                RootState::Absent(status.why())
            }
            (_, Err(why)) => RootState::Unreadable(why.clone()),
            (Some(FolderStatus::Unreadable(why)), Ok(())) => RootState::Unreadable(why.clone()),
            (Some(FolderStatus::Ok) | None, Ok(())) => RootState::Ok,
        }
    }
}

/// What a resolution knows of the library mounts: the registered mounts and the [`RootState`] of each one judged (a
/// registered mount not judged is usable). A first name of no registered mount is absent, [`NOT_REGISTERED`].
// ADR-0400 decision 19
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MountStates {
    mounts: Mounts,
    /// By name key.
    states: HashMap<PathKey, RootState>,
    /// How the items of an absent mount appear: `true` (a launch) — each as a ① `not-found` row, which the launch
    /// folds into one notice per mount; `false` (a read entrance) — one skipped ④ row stands for them.
    pub absent_items: bool,
}

impl MountStates {
    /// `mounts` registered, none judged yet — each usable until [`MountStates::set`].
    pub(crate) fn new(mounts: &Mounts, absent_items: bool) -> Self {
        MountStates {
            mounts: mounts.clone(),
            states: HashMap::new(),
            absent_items,
        }
    }

    /// Records the state of the registered mount `name`.
    pub(crate) fn set(&mut self, name: &MountName, state: RootState) {
        self.states.insert(name.key(), state);
    }

    /// The mount `path` lies in, as named (its registered name, or the path's first name), and its state.
    pub(crate) fn of(&self, path: &LibPath) -> (String, RootState) {
        match self.mounts.locate(path) {
            Some((mount, _)) => {
                let state = self.states.get(&mount.name.key());
                let state = state.cloned().unwrap_or(RootState::Ok);
                (mount.name.as_str().to_owned(), state)
            }
            None => {
                let first = path.as_str().split('/').next().unwrap_or_default();
                (
                    first.to_owned(),
                    RootState::Absent(NOT_REGISTERED.to_owned()),
                )
            }
        }
    }
}

/// Finds the items a resolution reaches. Every built-in is present ([`Looked::Present`] — a stand-in when it cannot
/// be read): only a library item can be [`Looked::Missing`].
pub trait Lookup {
    fn item(&self, r: &AssetRef) -> Looked<'_>;
}

pub enum Looked<'a> {
    Present(ItemView<'a>),
    Missing,
    /// It exists but cannot be read — the reason is a problem ② (e.g. `path-through-link`).
    Unreadable(Reason),
}

/// One item as [`Lookup`] holds it.
pub struct ItemView<'a> {
    pub kind: &'a ItemKind,
    pub content: &'a ItemContent,
    /// Its path on disk (a skill: its folder); `None` = an in-memory stand-in for a built-in that cannot be read — it
    /// loads nothing.
    pub file: Option<PathBuf>,
    /// Why this is a built-in's stand-in (`None` = not a stand-in) — reaching it is a `builtin-unavailable` notice.
    pub stand_in: Option<&'a str>,
}

/// What [`resolve`] answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolution {
    /// (source path as the preset spells it, disk path) of each instruction loaded, in expansion order — the priming
    /// excluded ([`Resolution::include_priming`]).
    pub instructions: Vec<(String, PathBuf)>,
    /// (name, skill folder on disk, via) of each skill loaded, in expansion order.
    pub skills: Vec<(String, PathBuf, Vec<String>)>,
    pub raw_settings: Vec<ResolvedSettings>,
    /// Folder entries in expansion order, each once by its normalized path's key.
    pub folders: Vec<ResolvedFolder>,
    /// The built-in priming was reached and its file exists — it goes first in the merged instructions.
    pub include_priming: bool,
    /// In expansion order; rows for unparsed attached paths after the rows of the expansion, then conflicts, folder rows
    /// (phase 2 — ① `folder-not-found` or ② `folder-unreadable`) last.
    pub problems: Vec<Problem>,
    pub notices: Vec<Notice>,
    /// The absent mounts items were reached in, in first-reach order: (the mount as named, why — [`NOT_REGISTERED`],
    /// `missing` or `not a folder`). Each one's ④ row, or its items' ① rows ([`MountStates::absent_items`]), are skipped.
    // ADR-0387
    // ADR-0400 decision 19
    pub absent_mounts: Vec<(String, String)>,
}

/// One settings value loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSettings {
    /// The settings asset it came from.
    pub asset: AssetRef,
    pub via: Vec<String>,
    /// For an agent, the value of one raw field named by its backend; for one preset on its own, an object of every
    /// raw field by name.
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFolder {
    /// Normalized absolute path.
    pub path: PathBuf,
    pub key: PathKey,
    pub via: Vec<String>,
    /// The line of its first-place entry in the last preset of `via`.
    pub line: u32,
    /// Where its instruction file goes in [`Resolution::instructions`]: the index right after the own instruction lines
    /// of the preset that lists it at its first place (parent TRD S25 §1-5 「at its place」).
    pub instructions_at: usize,
    /// Phase 2 — `None` until [`Resolution::attach_folder_status`] gets this folder's status.
    pub status: Option<FolderStatus>,
}

impl Resolution {
    /// Phase 1: the folders to stat, in expansion order.
    pub fn folder_paths(&self) -> Vec<PathBuf> {
        self.folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect()
    }

    /// Phase 2: records each folder's status from `statuses` (by key) and adds a row for every folder that is not
    /// `Ok` — on every backend and mode: ① [`FOLDER_NOT_FOUND`] for one that is missing or not a folder, ②
    /// [`FOLDER_UNREADABLE`] for one that exists but cannot be read or did not answer. A folder `statuses` lacks keeps
    /// `None` and gets no row.
    // ADR-0387
    pub fn attach_folder_status(&mut self, statuses: &HashMap<PathKey, FolderStatus>) {
        for folder in &mut self.folders {
            let Some(status) = statuses.get(&folder.key) else {
                continue;
            };
            folder.status = Some(status.clone());
            let path = folder.path.to_string_lossy();
            let line = Some(folder.line);
            let (item, via) = (folder_item(&folder.path), folder.via.clone());
            self.problems.push(match status {
                FolderStatus::Ok => continue,
                FolderStatus::Missing | FolderStatus::NotADirectory => BlockReason::Missing {
                    item,
                    via,
                    reason: reason(FOLDER_NOT_FOUND, json!({ "path": path }), line),
                },
                FolderStatus::Unreadable(why) => BlockReason::Broken {
                    item,
                    via,
                    reason: reason(FOLDER_UNREADABLE, json!({ "path": path, "why": why }), line),
                },
            });
        }
    }

    /// Whether a launch leaves `problem` out instead of refusing on it: every ① row, and the block-④ row of an absent
    /// mount. The split reads only the row's kind and, for a ④ row, whether its `mount` is in
    /// [`Resolution::absent_mounts`].
    // ADR-0387
    fn skips(&self, problem: &Problem) -> bool {
        match problem {
            BlockReason::Missing { .. } => true,
            BlockReason::LibraryUnavailable { reason } => {
                let mount = reason.args.get("mount").and_then(Value::as_str);
                mount.is_some_and(|mount| self.absent_mounts.iter().any(|(name, _)| name == mount))
            }
            BlockReason::Broken { .. } | BlockReason::Conflict { .. } => false,
        }
    }

    /// The rows a launch leaves out, in problem order. With [`Resolution::blocked`] they are exactly `problems`.
    pub fn skipped(&self) -> Vec<Problem> {
        self.problems
            .iter()
            .filter(|problem| self.skips(problem))
            .cloned()
            .collect()
    }

    /// The launch block: every problem a launch refuses on (all but [`Resolution::skipped`]), ordered by reason kind
    /// (② ③ ④ — TRD S25 §1-3 8), expansion order kept within a kind. `None` = nothing blocks.
    pub fn blocked(&self) -> Option<LaunchBlocked> {
        let mut reasons: Vec<Problem> = self
            .problems
            .iter()
            .filter(|problem| !self.skips(problem))
            .cloned()
            .collect();
        if reasons.is_empty() {
            return None;
        }
        reasons.sort_by_key(|problem| match problem {
            BlockReason::Missing { .. } => 0,
            BlockReason::Broken { .. } => 1,
            BlockReason::Conflict { .. } => 2,
            BlockReason::LibraryUnavailable { .. } => 3,
        });
        Some(LaunchBlocked { reasons })
    }
}

/// Resolves `target` over `lookup` (module header). A library item in a mount `target.mounts` does not hold usable is
/// not looked up — while a mount cannot be used its part of the catalog cannot be trusted: an absent mount's items are
/// skipped (one ④ row for the mount, or one ① row per item for a launch — [`MountStates::absent_items`]), and a mount
/// that exists but cannot be used is one ④ row naming it, which blocks.
// ADR-0361
// ADR-0387
// ADR-0400 decision 19
pub fn resolve(target: &Target, lookup: &dyn Lookup, backends: &dyn Backends) -> Resolution {
    let agent = match (&target.backend, target.mode) {
        (Some(word), Some(mode)) => backends.for_word(word).map(|check| Agent {
            check,
            mode,
            loads: check.loads(mode),
        }),
        _ => None,
    };
    let mut walk = Walk {
        lookup,
        backends,
        agent,
        states: &target.mounts,
        unusable: HashSet::new(),
        seen: HashSet::new(),
        seen_folders: HashSet::new(),
        loaded: Vec::new(),
        omitted_instructions: Vec::new(),
        omitted_skills: Vec::new(),
        out: Resolution::default(),
    };
    let default = AssetRef::BuiltIn(BuiltinId::DefaultPreset);
    let roots = target
        .default_preset
        .then_some(&default)
        .into_iter()
        .chain(&target.presets);
    for root in roots {
        walk.expand(root);
    }
    for (raw, why) in &target.unparsed {
        let args = json!({ "path": escape_controls(raw), "why": why.to_string() });
        walk.broken(raw.clone(), &[], reason(ITEM_PATH_INVALID, args, None));
    }
    walk.conflicts();
    walk.omitted_notice();
    walk.finish()
}

/// The catalog and the built-ins as a [`Lookup`] — a library item is present when the catalog holds it and its mount is
/// registered.
pub(crate) struct CatalogLookup<'a> {
    pub(crate) catalog: &'a Catalog,
    /// The mounts the catalog was scanned from — a library item's disk path is taken in its mount's folder.
    pub(crate) mounts: &'a Mounts,
    pub(crate) builtins: &'a Builtins,
}

impl Lookup for CatalogLookup<'_> {
    fn item(&self, r: &AssetRef) -> Looked<'_> {
        match r {
            AssetRef::BuiltIn(id) => Looked::Present(self.builtins.view(*id)),
            AssetRef::Library(path) => match self
                .catalog
                .get(&key(path))
                .and_then(|item| Some((item, self.mounts.disk_path(&item.path)?)))
            {
                Some((item, file)) => Looked::Present(ItemView {
                    kind: &item.kind,
                    content: &item.content,
                    file: Some(file),
                    stand_in: None,
                }),
                None => Looked::Missing,
            },
        }
    }
}

/// Which backend tags a resolution takes — one rule for the resolution and for what is read or stat'ed ahead of it
/// (a launch's overlay, a read entrance's mount stats), so neither follows an entry the resolution leaves out: one
/// preset on its own takes every tag; an agent only its backend's.
#[derive(Clone, Copy)]
pub(crate) struct TagRule<'l> {
    backends: &'l dyn Backends,
    /// The agent's backend; `None` = one preset on its own.
    backend: Option<&'static str>,
}

impl<'l> TagRule<'l> {
    /// The rule of a resolution for `backend` in `mode`, as [`resolve`] reads its target: an agent when both are given
    /// and the backend has a settings check; else one preset on its own.
    pub(crate) fn of(
        backends: &'l dyn Backends,
        backend: Option<&str>,
        mode: Option<AgentOutputFormat>,
    ) -> Self {
        let backend = match (backend, mode) {
            (Some(word), Some(_)) => backends.for_word(word).map(|check| check.backend()),
            _ => None,
        };
        TagRule { backends, backend }
    }

    /// Whether a tag or raw field named `word` belongs to this resolution.
    pub(crate) fn takes(&self, word: &str) -> bool {
        match self.backend {
            None => true,
            Some(backend) => self
                .backends
                .for_word(word)
                .is_some_and(|check| check.backend() == backend),
        }
    }

    /// The library references of `doc` a resolution under this rule follows, in [`crate::catalog::preset_refs`] order: every
    /// `presets` and `settings` entry (a tag in `settings` is a line problem — the item reads as common), and each
    /// `instructions` and `skills` entry that is common or carries a tag this rule takes.
    pub(crate) fn refs<'d>(&self, doc: &'d PresetDoc) -> Vec<&'d LibPath> {
        let taken = |item: &&PresetItem| item.backend.as_deref().is_none_or(|tag| self.takes(tag));
        doc.presets
            .iter()
            .chain(doc.instructions.iter().filter(taken))
            .chain(doc.skills.iter().filter(taken))
            .chain(&doc.settings)
            .filter_map(|item| match &item.target {
                AssetRef::Library(path) => Some(path),
                AssetRef::BuiltIn(_) => None,
            })
            .collect()
    }
}

/// The agent a resolution is for: its backend's check, its mode and what that mode loads.
#[derive(Clone, Copy)]
struct Agent<'l> {
    check: &'l dyn SettingsCheck,
    mode: AgentOutputFormat,
    loads: ModeLoads,
}

/// The identity an expansion dedups by: a library item by its catalog key, a built-in by its id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RefKey {
    Library(PathKey),
    BuiltIn(BuiltinId),
}

pub(crate) fn ref_key(r: &AssetRef) -> RefKey {
    match r {
        AssetRef::Library(path) => RefKey::Library(key(path)),
        AssetRef::BuiltIn(id) => RefKey::BuiltIn(*id),
    }
}

/// How a resolution may use a library item's mount ([`MountStates`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MountUse {
    Usable,
    /// Not registered or not there — its items are skipped.
    Absent,
    /// There but cannot be read — its items block.
    Unusable,
}

/// A preset being expanded.
struct Frame<'l> {
    key: RefKey,
    /// A library preset (its `presets` may not list a built-in).
    library: bool,
    doc: &'l PresetDoc,
    /// The next entry of `doc.presets` to expand.
    next: usize,
    /// The chain down to and including this preset.
    via: Vec<String>,
}

/// A skill the expansion loads.
struct LoadedSkill {
    at: RefKey,
    name: String,
    key: String,
    spelling: String,
    file: PathBuf,
    via: Vec<String>,
    /// The backend word of each tag it is listed under; `None` = listed as common.
    tags: BTreeSet<Option<&'static str>>,
}

struct Walk<'l> {
    lookup: &'l dyn Lookup,
    backends: &'l dyn Backends,
    agent: Option<Agent<'l>>,
    states: &'l MountStates,
    /// The mounts whose ④ row is recorded, by name key.
    unusable: HashSet<PathKey>,
    /// Per list: an item listed again in the same list stays at its first place; in another list it is judged there.
    seen: HashSet<(ListName, RefKey)>,
    seen_folders: HashSet<PathKey>,
    loaded: Vec<LoadedSkill>,
    /// Library instructions this mode does not load, as spelled.
    omitted_instructions: Vec<String>,
    /// Names of skills this mode does not load.
    omitted_skills: Vec<String>,
    out: Resolution,
}

impl<'l> Walk<'l> {
    /// Expands one attached preset, depth first.
    // An explicit stack, not recursion: a long chain of nested presets would overflow the thread's stack, which aborts
    //   the process.
    fn expand(&mut self, root: &AssetRef) {
        let mut stack: Vec<Frame<'l>> = Vec::new();
        if let Some(frame) = self.open(root, None, &[], &stack) {
            stack.push(frame);
        }
        while let Some(top) = stack.last_mut() {
            let doc: &'l PresetDoc = top.doc;
            match doc.presets.get(top.next) {
                Some(entry) => {
                    top.next += 1;
                    let (via, library) = (top.via.clone(), top.library);
                    match &entry.target {
                        AssetRef::BuiltIn(_) if library => {
                            let spelling = entry.target.spelling();
                            let args = json!({ "path": spelling });
                            let refused = reason(BUILTIN_IN_LIBRARY_PRESET, args, Some(entry.line));
                            self.broken(spelling, &via, refused);
                        }
                        target => {
                            if let Some(frame) = self.open(target, Some(entry.line), &via, &stack) {
                                stack.push(frame);
                            }
                        }
                    }
                }
                None => {
                    if let Some(frame) = stack.pop() {
                        self.own_lists(&frame);
                    }
                }
            }
        }
    }

    /// The frame of preset `at` (listed at `line` of the last preset in `via`), or `None` with its problem recorded —
    /// or nothing, when it was expanded before.
    fn open(
        &mut self,
        at: &AssetRef,
        line: Option<u32>,
        via: &[String],
        stack: &[Frame<'l>],
    ) -> Option<Frame<'l>> {
        let at_key = ref_key(at);
        if stack.iter().any(|frame| frame.key == at_key) {
            let spelling = at.spelling();
            let args = json!({ "path": spelling });
            self.broken(spelling, via, reason(PRESET_CYCLE, args, line));
            return None;
        }
        let (spelling, view) = self.look(ListName::Presets, at, line, via)?;
        let ItemContent::Preset { doc, .. } = view.content else {
            self.wrong_kind(spelling, via, ListName::Presets, line);
            return None;
        };
        for problem in &doc.line_problems {
            self.broken(spelling.clone(), via, problem.clone());
        }
        let mut chain = via.to_vec();
        chain.push(spelling);
        Some(Frame {
            key: at_key,
            library: matches!(at, AssetRef::Library(_)),
            doc,
            next: 0,
            via: chain,
        })
    }

    /// The item `at` points at, once per list in a resolution (a later reference to it from the same list is
    /// skipped). A missing, unreadable or broken item records its problem and answers `None`.
    fn look(
        &mut self,
        list: ListName,
        at: &AssetRef,
        line: Option<u32>,
        via: &[String],
    ) -> Option<(String, ItemView<'l>)> {
        if !self.seen.insert((list, ref_key(at))) {
            return None;
        }
        let spelling = at.spelling();
        if let AssetRef::Library(path) = at {
            match self.mount_use(path) {
                MountUse::Usable => {}
                MountUse::Absent if self.states.absent_items => {
                    self.not_found(spelling, via, line);
                    return None;
                }
                MountUse::Absent | MountUse::Unusable => return None,
            }
        }
        let lookup: &'l dyn Lookup = self.lookup;
        match lookup.item(at) {
            Looked::Missing => {
                self.not_found(spelling, via, line);
                None
            }
            Looked::Unreadable(why) => {
                self.broken(spelling, via, why);
                None
            }
            Looked::Present(view) => {
                if let Some(why) = view.stand_in {
                    let args = json!({ "path": spelling, "why": why });
                    let told = reason(BUILTIN_UNAVAILABLE, args, None);
                    // Once per built-in. A second reach in one resolution is always through a list of the wrong
                    //   kind (a wrong-kind row blocks it); the notice is still told once.
                    if !self.out.notices.contains(&told) {
                        self.out.notices.push(told);
                    }
                }
                match view.content {
                    ItemContent::Broken { reason: why } => {
                        self.broken(spelling, via, why.clone());
                        None
                    }
                    _ => Some((spelling, view)),
                }
            }
        }
    }

    /// How `path`'s mount may be used. On the first reach of a mount that cannot be, its state is recorded: an absent
    /// mount in [`Resolution::absent_mounts`], and — but for an absent mount of a launch, whose items are ① rows — one
    /// ④ row naming it (`args` = `{mount, message}`).
    // ADR-0400 decision 19
    fn mount_use(&mut self, path: &LibPath) -> MountUse {
        let (mount, state) = self.states.of(path);
        let (why, used) = match state {
            RootState::Ok => return MountUse::Usable,
            RootState::Absent(why) => (why, MountUse::Absent),
            RootState::Unreadable(why) => (why, MountUse::Unusable),
        };
        if !self.unusable.insert(path_key(Path::new(&mount))) {
            return used;
        }
        if used == MountUse::Absent {
            self.out.absent_mounts.push((mount.clone(), why.clone()));
            if self.states.absent_items {
                return used;
            }
        }
        let args = json!({ "mount": mount, "message": why });
        self.out.problems.push(BlockReason::LibraryUnavailable {
            reason: reason(LIBRARY_UNAVAILABLE, args, None),
        });
        used
    }

    /// A ① `not-found` row for the reference `spelling` (`args` = `{path}`).
    fn not_found(&mut self, spelling: String, via: &[String], line: Option<u32>) {
        let args = json!({ "path": spelling });
        self.out.problems.push(BlockReason::Missing {
            item: spelling,
            via: via.to_vec(),
            reason: reason(NOT_FOUND, args, line),
        });
    }

    fn own_lists(&mut self, frame: &Frame<'l>) {
        let doc = frame.doc;
        for entry in self.in_tag_order(&doc.instructions) {
            self.instruction(entry, &frame.via);
        }
        // A tag in `settings` is a line problem; the item reads as common.
        for entry in &doc.settings {
            self.settings(entry, &frame.via);
        }
        for entry in self.in_tag_order(&doc.skills) {
            self.skill(entry, &frame.via);
        }
        for folder in &doc.folders {
            self.folder(folder, &frame.via);
        }
    }

    /// The items of one list this resolution takes, in load order: the common ones, then the tagged ones it loads.
    fn in_tag_order<'d>(&self, items: &'d [PresetItem]) -> Vec<&'d PresetItem> {
        let common = items.iter().filter(|item| item.backend.is_none());
        let tagged = items.iter().filter(|item| {
            item.backend
                .as_deref()
                .is_some_and(|tag| self.takes_backend(tag))
        });
        common.chain(tagged).collect()
    }

    /// Whether a tag or raw field named `word` belongs to this resolution ([`TagRule::takes`]).
    fn takes_backend(&self, word: &str) -> bool {
        TagRule {
            backends: self.backends,
            backend: self.agent.map(|agent| agent.check.backend()),
        }
        .takes(word)
    }

    fn instruction(&mut self, entry: &PresetItem, via: &[String]) {
        let line = Some(entry.line);
        let Some((spelling, view)) = self.look(ListName::Instructions, &entry.target, line, via)
        else {
            return;
        };
        let ItemContent::Instruction { .. } = view.content else {
            return self.wrong_kind(spelling, via, ListName::Instructions, line);
        };
        let Some(file) = view.file else {
            return;
        };
        if entry.target == AssetRef::BuiltIn(BuiltinId::Priming) {
            self.out.include_priming = true;
            return;
        }
        match self.agent.map(|agent| agent.loads.instructions) {
            Some(InstructionLoad::BuiltinPrimingOnly) => self.omitted_instructions.push(spelling),
            Some(InstructionLoad::All) | None => self.out.instructions.push((spelling, file)),
        }
    }

    fn settings(&mut self, entry: &PresetItem, via: &[String]) {
        let line = Some(entry.line);
        let Some((spelling, view)) = self.look(ListName::Settings, &entry.target, line, via) else {
            return;
        };
        let ItemContent::Settings { doc, verdict } = view.content else {
            return self.wrong_kind(spelling, via, ListName::Settings, line);
        };
        // One preset on its own shows every field; an agent loads only its backend's, and only a refusal in that
        //   field blocks it (a refusal in another backend's field shows on the asset's own row).
        let values: Vec<Value> = match self.agent {
            None => vec![Value::Object(
                doc.raw
                    .iter()
                    .map(|field| (field.name.clone(), field.value.clone()))
                    .collect(),
            )],
            Some(_) => doc
                .raw
                .iter()
                .filter(|field| self.takes_backend(&field.name))
                .map(|field| field.value.clone())
                .collect(),
        };
        let refusals: Vec<Reason> = verdict
            .problems
            .iter()
            .filter(|(field, _)| self.takes_backend(field))
            .map(|(_, refusal)| refusal.clone())
            .collect();
        for value in values {
            self.out.raw_settings.push(ResolvedSettings {
                asset: entry.target.clone(),
                via: via.to_vec(),
                value,
            });
        }
        for refusal in refusals {
            self.broken(spelling.clone(), via, refusal);
        }
    }

    fn skill(&mut self, entry: &PresetItem, via: &[String]) {
        let line = Some(entry.line);
        let tag = entry
            .backend
            .as_deref()
            .and_then(|word| self.backends.for_word(word))
            .map(|check| check.backend());
        let at = ref_key(&entry.target);
        // A skill listed again keeps its first place and joins the backends it is listed for there (U5).
        if let Some(loaded) = self.loaded.iter_mut().find(|skill| skill.at == at) {
            loaded.tags.insert(tag);
            return;
        }
        let Some((spelling, view)) = self.look(ListName::Skills, &entry.target, line, via) else {
            return;
        };
        let ItemContent::Skill {
            name,
            file_name,
            folder,
            text,
            ..
        } = view.content
        else {
            return self.wrong_kind(spelling, via, ListName::Skills, line);
        };
        let Some(file) = view.file else {
            return;
        };
        if let Some(agent) = self.agent {
            if !agent.loads.skills {
                self.omitted_skills.push(name.clone());
                return;
            }
            let refusal = match check_skill(agent.check, agent.mode, file_name, folder, text) {
                SkillFormat::Invalid(refusal) => Some(refusal),
                SkillFormat::Valid { .. } | SkillFormat::NotChecked => {
                    (!agent.check.accepts_skill_name(name, agent.mode))
                        .then(|| reason(SKILL_NAME_INVALID, json!({ "name": name }), None))
                }
            };
            if let Some(refusal) = refusal {
                self.broken(spelling.clone(), via, refusal);
            }
        }
        self.loaded.push(LoadedSkill {
            at,
            name: name.clone(),
            key: conflict_key(name),
            spelling,
            file,
            via: via.to_vec(),
            tags: BTreeSet::from([tag]),
        });
    }

    fn folder(&mut self, folder: &FolderRef, via: &[String]) {
        // A relative entry is the preset's line problem already.
        let Ok(path) = &folder.normalized else {
            return;
        };
        let folder_key = path_key(path);
        if !self.seen_folders.insert(folder_key.clone()) {
            return;
        }
        if let Some(agent) = self.agent {
            if agent.check.folder_entry_blocks(path) {
                let args = json!({ "path": path.to_string_lossy() });
                let refused = reason(FOLDER_PATH_CLI_UNSAFE, args, Some(folder.line));
                self.broken(folder_item(path), via, refused);
            }
        }
        self.out.folders.push(ResolvedFolder {
            path: path.clone(),
            key: folder_key,
            via: via.to_vec(),
            line: folder.line,
            // `own_lists` takes a preset's folders after its instructions, so this is the end of its own lines.
            instructions_at: self.out.instructions.len(),
            status: None,
        });
    }

    /// One row per collision key among the loaded skills that collide. For an agent every loaded skill counts; for one
    /// preset on its own a pair collides only when some backend would load both — common with common, common with a
    /// tag, and one tag with the same tag (U5 (a)); every registered backend is asked, so a new one is covered.
    fn conflicts(&mut self) {
        let loaded = &self.loaded;
        let everyone: Vec<Vec<usize>> = match self.agent {
            Some(_) => vec![(0..loaded.len()).collect()],
            None => self
                .backends
                .all()
                .iter()
                .map(|check| {
                    (0..loaded.len())
                        .filter(|&i| {
                            loaded[i]
                                .tags
                                .iter()
                                .any(|tag| tag.is_none_or(|tag| tag == check.backend()))
                        })
                        .collect()
                })
                .collect(),
        };
        let mut colliding: HashMap<&str, BTreeSet<usize>> = HashMap::new();
        for members in &everyone {
            let mut by_key: HashMap<&str, Vec<usize>> = HashMap::new();
            for &i in members {
                by_key.entry(loaded[i].key.as_str()).or_default().push(i);
            }
            for (collision, group) in by_key {
                if group.len() >= 2 {
                    colliding.entry(collision).or_default().extend(group);
                }
            }
        }
        let mut told = HashSet::new();
        for skill in loaded {
            let collision = skill.key.as_str();
            let Some(group) = colliding.get(collision) else {
                continue;
            };
            if told.insert(collision) {
                self.out.problems.push(BlockReason::Conflict {
                    reason: reason(SKILL_NAME_CONFLICT, json!({ "name": collision }), None),
                    items: group
                        .iter()
                        .map(|&i| (loaded[i].spelling.clone(), loaded[i].via.clone()))
                        .collect(),
                });
            }
        }
    }

    /// The agent's `omitted` notice, when its mode leaves out instructions or skills it would otherwise load and the
    /// backend gives a code for it (`args` = `{instructions: [paths], skills: [names]}`).
    fn omitted_notice(&mut self) {
        let Some(agent) = self.agent else {
            return;
        };
        if self.omitted_instructions.is_empty() && self.omitted_skills.is_empty() {
            return;
        }
        if let Some(code) = agent.check.notice_codes(agent.mode).omitted {
            let args = json!({
                "instructions": self.omitted_instructions,
                "skills": self.omitted_skills,
            });
            self.out.notices.push(reason(code, args, None));
        }
    }

    fn finish(self) -> Resolution {
        let mut out = self.out;
        out.skills = self
            .loaded
            .into_iter()
            .map(|skill| (skill.name, skill.file, skill.via))
            .collect();
        out
    }

    fn broken(&mut self, item: String, via: &[String], why: Reason) {
        self.out.problems.push(BlockReason::Broken {
            item,
            via: via.to_vec(),
            reason: why,
        });
    }

    /// An entry of `list` that points at an item of another kind (`args` = `{path, list}`).
    fn wrong_kind(&mut self, spelling: String, via: &[String], list: ListName, line: Option<u32>) {
        let args = json!({ "path": spelling, "list": list.key() });
        self.broken(spelling, via, reason(WRONG_KIND, args, line));
    }
}

fn reason(code: &str, args: Value, line: Option<u32>) -> Reason {
    Reason {
        code: code.to_owned(),
        args,
        line,
    }
}

/// The `item` of a folder entry's problem, and the `path` of its instruction source (`agent::launch::SourceRef`).
pub(crate) fn folder_item(path: &Path) -> String {
    format!("{FOLDER_ITEM_PREFIX}{}", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::tests::full;
    use crate::catalog::harness::Lib;
    use crate::mounts::{Mount, MountName};
    use engram_dashboard_platform::fs::normalize_folder;

    const T: AgentOutputFormat = AgentOutputFormat::Terminal;
    const J: AgentOutputFormat = AgentOutputFormat::StreamJson;

    fn preset(lists: &str) -> String {
        format!("kind: preset\nversion: 1\n{lists}")
    }

    fn skill(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\nbody\n")
    }

    fn refs(presets: &[&str]) -> Vec<AssetRef> {
        presets
            .iter()
            .map(|text| AssetRef::parse(text).unwrap())
            .collect()
    }

    fn agent(word: &str, mode: AgentOutputFormat, presets: &[&str]) -> Target {
        Target {
            backend: Some(word.to_owned()),
            mode: Some(mode),
            default_preset: false,
            presets: refs(presets),
            unparsed: Vec::new(),
            mounts: usable(),
        }
    }

    fn alone(preset: &str) -> Target {
        Target {
            backend: None,
            mode: None,
            default_preset: false,
            presets: refs(&[preset]),
            unparsed: Vec::new(),
            mounts: usable(),
        }
    }

    /// The mount `library` (the harness's — `/lib`) and `team` (`/team`, no item in the catalog) registered, each in
    /// the state given (`None` = not judged: usable); `absent_items` as a launch resolves.
    fn states(
        library: Option<RootState>,
        team: Option<RootState>,
        absent_items: bool,
    ) -> MountStates {
        let team_name = MountName::parse("team").unwrap();
        let mounts = Mounts::new(vec![
            Mount {
                name: MountName::library(),
                folder: PathBuf::from("/lib"),
            },
            Mount {
                name: team_name.clone(),
                folder: PathBuf::from("/team"),
            },
        ])
        .unwrap();
        let mut states = MountStates::new(&mounts, absent_items);
        if let Some(state) = library {
            states.set(&MountName::library(), state);
        }
        if let Some(state) = team {
            states.set(&team_name, state);
        }
        states
    }

    /// Every registered mount usable, as a read entrance.
    fn usable() -> MountStates {
        states(None, None, false)
    }

    /// The ④ row of `mount` (`args` = `{mount, message}`).
    fn block_four(mount: &str, message: &str) -> BlockReason {
        BlockReason::LibraryUnavailable {
            reason: reason(
                LIBRARY_UNAVAILABLE,
                json!({ "mount": mount, "message": message }),
                None,
            ),
        }
    }

    /// A library holding `files` (relative path, text), scanned.
    fn library(files: &[(&str, &str)]) -> Lib {
        let mut lib = Lib::new();
        for (rel, text) in files {
            lib.store.file(rel, text);
        }
        lib.scan(None);
        lib
    }

    fn resolve_with(lib: &Lib, builtins: &Builtins, target: &Target) -> Resolution {
        let mounts = Mounts::new(vec![Mount {
            name: lib.mount.clone(),
            folder: lib.root.clone(),
        }])
        .unwrap();
        let lookup = CatalogLookup {
            catalog: &lib.catalog,
            mounts: &mounts,
            builtins,
        };
        resolve(target, &lookup, &lib.backends)
    }

    /// `resolve` with no built-in folder.
    fn run(lib: &Lib, target: &Target) -> Resolution {
        resolve_with(lib, &Builtins::new(None, &lib.backends), target)
    }

    fn instructions(resolution: &Resolution) -> Vec<&str> {
        resolution
            .instructions
            .iter()
            .map(|(path, _)| path.as_str())
            .collect()
    }

    fn skill_names(resolution: &Resolution) -> Vec<&str> {
        resolution
            .skills
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect()
    }

    /// `(item, code)` of every problem; a conflict as (its paths joined by `+`, code); block ④ with an empty item.
    fn problems(found: &[Problem]) -> Vec<(String, String)> {
        found
            .iter()
            .map(|problem| match problem {
                BlockReason::Missing { item, reason, .. }
                | BlockReason::Broken { item, reason, .. } => (item.clone(), reason.code.clone()),
                BlockReason::Conflict { reason, items } => {
                    let paths: Vec<&str> = items.iter().map(|(path, _)| path.as_str()).collect();
                    (paths.join("+"), reason.code.clone())
                }
                BlockReason::LibraryUnavailable { reason } => (String::new(), reason.code.clone()),
            })
            .collect()
    }

    fn row(item: &str, code: &str) -> (String, String) {
        (item.to_owned(), code.to_owned())
    }

    fn via(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    /// An absolute folder path on this OS (under the temp folder), spelled with `/` after it.
    fn absolute(rel: &str) -> String {
        format!("{}/{rel}", std::env::temp_dir().display())
    }

    fn normalized(spelled: &str) -> PathBuf {
        normalize_folder(Path::new(spelled)).unwrap()
    }

    // ── expansion ────────────────────────────────────────────────────────────────────────────────────────

    // Parent §1-3 2: [A, B] attached, A holds C, C and B share x — x loads once, at C's place.
    #[test]
    fn nested_presets_expand_first_and_each_path_stays_at_its_first_place() {
        let lib = library(&[
            (
                "A.yaml",
                &preset("presets:\n  - library/C.yaml\ninstructions:\n  - library/a.md\n"),
            ),
            (
                "B.yaml",
                &preset("presets:\n  - library/C.yaml\ninstructions:\n  - library/x.md\n  - library/b.md\n"),
            ),
            ("C.yaml", &preset("instructions:\n  - library/x.md\n  - library/c.md\n")),
            ("a.md", "a"),
            ("b.md", "b"),
            ("c.md", "c"),
            ("x.md", "x"),
        ]);
        let resolution = run(
            &lib,
            &agent("claude", J, &["library/A.yaml", "library/B.yaml"]),
        );
        assert_eq!(
            instructions(&resolution),
            [
                "library/x.md",
                "library/c.md",
                "library/a.md",
                "library/b.md"
            ]
        );
        assert_eq!(
            resolution.instructions[0].1,
            Path::new("/lib").join("x.md"),
            "the disk path under the root"
        );
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
    }

    // Parent §1-3 1 (G51): [library/A, library/B], A = [common a1 · codex a2], B = [common b1 · codex b2]; A holds C.
    #[test]
    fn tagged_items_follow_their_own_presets_common_items() {
        let lib = library(&[
            (
                "A.yaml",
                &preset("presets:\n  - library/C.yaml\ninstructions:\n  - library/a1.md\n  - {path: library/a2.md, backend: codex}\n"),
            ),
            (
                "B.yaml",
                &preset("instructions:\n  - {path: library/b2.md, backend: codex}\n  - library/b1.md\n"),
            ),
            (
                "C.yaml",
                &preset("instructions:\n  - {path: library/c2.md, backend: Codex}\n  - library/c1.md\n"),
            ),
            ("a1.md", "1"),
            ("a2.md", "2"),
            ("b1.md", "1"),
            ("b2.md", "2"),
            ("c1.md", "1"),
            ("c2.md", "2"),
        ]);
        let codex = run(
            &lib,
            &agent("codex", J, &["library/A.yaml", "library/B.yaml"]),
        );
        assert_eq!(
            instructions(&codex),
            [
                "library/c1.md",
                "library/c2.md",
                "library/a1.md",
                "library/a2.md",
                "library/b1.md",
                "library/b2.md"
            ]
        );
        let claude = run(
            &lib,
            &agent("claude", J, &["library/A.yaml", "library/B.yaml"]),
        );
        assert_eq!(
            instructions(&claude),
            ["library/c1.md", "library/a1.md", "library/b1.md"]
        );
    }

    #[test]
    fn a_preset_reached_again_while_it_expands_is_a_cycle_on_that_preset() {
        let lib = library(&[
            (
                "A.yaml",
                &preset("presets:\n  - library/B.yaml\ninstructions:\n  - library/a.md\n"),
            ),
            (
                "B.yaml",
                &preset("instructions:\n  - library/b.md\npresets:\n  - library/A.yaml\n"),
            ),
            ("a.md", "a"),
            ("b.md", "b"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["library/A.yaml"]));
        assert_eq!(
            instructions(&resolution),
            ["library/b.md", "library/a.md"],
            "each once"
        );
        assert_eq!(
            resolution.problems,
            [BlockReason::Broken {
                item: "library/A.yaml".to_owned(),
                via: via(&["library/A.yaml", "library/B.yaml"]),
                reason: reason(PRESET_CYCLE, json!({ "path": "library/A.yaml" }), Some(6)),
            }]
        );
    }

    #[test]
    fn missing_broken_wrong_kind_and_line_problems_are_rows_with_their_chain() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - library/N.yaml\n  - {path: library/Q.yaml, backend: codex}\n  - library/bad.yaml\n"),
            ),
            (
                "N.yaml",
                &preset("instructions:\n  - library/gone.md\nskills:\n  - library/a.md\n"),
            ),
            ("Q.yaml", &preset("")),
            ("bad.yaml", "kind: preset\nversion: [\n"),
            ("a.md", "a"),
        ]);
        let resolution = run(
            &lib,
            &agent("claude", J, &["library/P.yaml", "library/absent.yaml"]),
        );
        assert_eq!(
            problems(&resolution.problems),
            [
                row("library/P.yaml", "backend-tag-not-allowed"),
                row("library/gone.md", NOT_FOUND),
                row("library/a.md", WRONG_KIND),
                row("library/bad.yaml", "yaml-syntax"),
                row("library/absent.yaml", NOT_FOUND),
            ]
        );
        let BlockReason::Missing {
            via: chain,
            reason: why,
            ..
        } = &resolution.problems[1]
        else {
            panic!("{:?}", resolution.problems[1])
        };
        assert_eq!(*chain, via(&["library/P.yaml", "library/N.yaml"]));
        assert_eq!(why.args, json!({ "path": "library/gone.md" }));
        assert_eq!(why.line, Some(4), "the line of the entry that names it");
        let BlockReason::Missing {
            via: chain,
            reason: why,
            ..
        } = &resolution.problems[4]
        else {
            panic!("{:?}", resolution.problems[4])
        };
        assert!(chain.is_empty(), "an attached preset has no chain");
        assert_eq!(why.line, None);
    }

    #[test]
    fn a_builtin_in_a_library_presets_presets_is_broken_but_an_agent_may_attach_it() {
        let dir = full();
        let lib = library(&[("P.yaml", &preset("presets:\n  - engram/default\n"))]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let resolution = resolve_with(&lib, &builtins, &agent("claude", J, &["library/P.yaml"]));
        assert_eq!(
            resolution.problems,
            [BlockReason::Broken {
                item: "engram/default".to_owned(),
                via: via(&["library/P.yaml"]),
                reason: reason(
                    BUILTIN_IN_LIBRARY_PRESET,
                    json!({ "path": "engram/default" }),
                    Some(4)
                ),
            }]
        );
        assert!(!resolution.include_priming);
        let attached = resolve_with(&lib, &builtins, &agent("claude", J, &["engram/default"]));
        assert!(attached.problems.is_empty(), "{:?}", attached.problems);
        assert!(attached.include_priming);
    }

    // ── built-ins ────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_default_preset_brings_the_priming_and_its_settings_to_the_backend_that_has_a_field() {
        let dir = full();
        let lib = library(&[]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("claude", J, &[]);
        target.default_preset = true;
        let claude = resolve_with(&lib, &builtins, &target);
        assert!(claude.include_priming);
        assert!(
            claude.instructions.is_empty(),
            "the priming is not a source"
        );
        assert_eq!(
            claude.raw_settings,
            [ResolvedSettings {
                asset: AssetRef::BuiltIn(BuiltinId::DefaultSettings),
                via: via(&["engram/default"]),
                value: json!({ "env": { "DISABLE_AUTOUPDATER": "1" } }),
            }]
        );
        target.backend = Some("codex".to_owned());
        let codex = resolve_with(&lib, &builtins, &target);
        assert!(codex.include_priming);
        assert!(codex.raw_settings.is_empty(), "no codex field");
        assert!(codex.notices.is_empty() && codex.problems.is_empty());
    }

    #[test]
    fn a_missing_builtin_folder_means_no_priming_and_no_block() {
        let lib = library(&[("P.yaml", &preset("instructions:\n  - engram/priming\n"))]);
        let mut target = agent("claude", J, &["library/P.yaml"]);
        target.default_preset = true;
        let resolution = run(&lib, &target);
        assert!(!resolution.include_priming);
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert!(resolution.raw_settings.is_empty() && resolution.instructions.is_empty());
        let told = |path: &str| {
            let args = json!({ "path": path, "why": "no built-in folder" });
            reason(BUILTIN_UNAVAILABLE, args, None)
        };
        assert_eq!(
            resolution.notices,
            [told("engram/default"), told("engram/priming")],
            "each built-in reached, in expansion order"
        );
    }

    #[test]
    fn a_default_preset_that_does_not_parse_runs_empty_with_one_notice() {
        let dir = crate::builtin::tests::folder(&[
            (
                crate::builtin::file_of(BuiltinId::DefaultPreset),
                "kind: preset\nversion: [\n",
            ),
            (
                crate::builtin::file_of(BuiltinId::Priming),
                crate::builtin::tests::PRIMING,
            ),
            (
                crate::builtin::file_of(BuiltinId::DefaultSettings),
                crate::builtin::tests::DEFAULT_SETTINGS,
            ),
        ]);
        let lib = library(&[]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("claude", J, &[]);
        target.default_preset = true;
        let resolution = resolve_with(&lib, &builtins, &target);
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert!(!resolution.include_priming, "the stand-in lists nothing");
        let [notice] = resolution.notices.as_slice() else {
            panic!("{:?}", resolution.notices)
        };
        assert_eq!(notice.code, BUILTIN_UNAVAILABLE);
        assert_eq!(notice.args["path"], "engram/default");
        let why = notice.args["why"].as_str().unwrap();
        assert!(why.starts_with("yaml-syntax: "), "{why}");
    }

    #[test]
    fn a_stand_in_is_told_once_when_reached_and_not_at_all_when_unreached() {
        let dir = crate::builtin::tests::folder(&[
            (
                crate::builtin::file_of(BuiltinId::DefaultPreset),
                "kind: preset\nversion: 1\ninstructions:\n  - engram/priming\n",
            ),
            (
                crate::builtin::file_of(BuiltinId::Priming),
                crate::builtin::tests::PRIMING,
            ),
            (
                crate::builtin::file_of(BuiltinId::DefaultSettings),
                "kind: settings\nversion: [\n",
            ),
        ]);
        let lib = library(&[(
            "P.yaml",
            &preset("settings:\n  - engram/settings/default\ninstructions:\n  - engram/settings/default\n"),
        )]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut unreached = agent("claude", J, &[]);
        unreached.default_preset = true;
        let resolution = resolve_with(&lib, &builtins, &unreached);
        assert!(resolution.include_priming);
        assert!(resolution.notices.is_empty() && resolution.problems.is_empty());
        let twice = resolve_with(&lib, &builtins, &agent("claude", J, &["library/P.yaml"]));
        let told: Vec<&Value> = twice.notices.iter().map(|n| &n.args["path"]).collect();
        assert_eq!(told, ["engram/settings/default"], "reached from two lists");
        assert_eq!(
            problems(&twice.problems),
            [row("engram/settings/default", WRONG_KIND)],
            "the stand-in is a settings item, not an instruction"
        );
    }

    // ── skills: conflicts, names and formats ─────────────────────────────────────────────────────────────

    #[test]
    fn conflicts_count_only_the_skills_the_agent_loads_and_carry_every_item_with_its_chain() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - library/N.yaml\nskills:\n  - library/s1\n  - {path: library/s3, backend: codex}\n  - {path: library/s4, backend: claude}\n"),
            ),
            ("N.yaml", &preset("skills:\n  - library/s2\n")),
            ("s1/SKILL.md", &skill("dup", "one")),
            ("s2/SKILL.md", &skill("Dup", "two")),
            ("s3/SKILL.md", &skill("dup", "three")),
            ("s4/SKILL.md", &skill("solo", "four")),
        ]);
        let claude = run(&lib, &agent("claude", T, &["library/P.yaml"]));
        assert_eq!(skill_names(&claude), ["Dup", "dup", "solo"]);
        assert_eq!(
            claude.problems,
            [BlockReason::Conflict {
                reason: reason(SKILL_NAME_CONFLICT, json!({ "name": "dup" }), None),
                items: vec![
                    (
                        "library/s2".to_owned(),
                        via(&["library/P.yaml", "library/N.yaml"])
                    ),
                    ("library/s1".to_owned(), via(&["library/P.yaml"])),
                ],
            }],
            "the codex-tagged s3 is not loaded"
        );
        let codex_json = run(&lib, &agent("codex", J, &["library/P.yaml"]));
        let conflicts: Vec<_> = problems(&codex_json.problems)
            .into_iter()
            .filter(|(_, code)| code == SKILL_NAME_CONFLICT)
            .collect();
        assert_eq!(
            conflicts,
            [row("library/s2+library/s1+library/s3", SKILL_NAME_CONFLICT)]
        );
        let codex_terminal = run(&lib, &agent("codex", T, &["library/P.yaml"]));
        assert!(
            codex_terminal.skills.is_empty(),
            "the terminal loads no skills"
        );
        assert!(
            codex_terminal.problems.is_empty(),
            "nothing loaded, nothing to count"
        );
    }

    #[test]
    fn the_skill_format_and_name_are_judged_through_the_agents_port_only() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("skills:\n  - library/empty\n  - library/Upper\n  - {path: library/odd, backend: gamma}\n"),
            ),
            (
                "empty/SKILL.md",
                "---\nname: empty\ndescription: \"\"\n---\n",
            ),
            ("Upper/SKILL.md", &skill("Upper", "fine")),
            ("odd/SKILL.md", &skill("odd", "please refuse me")),
        ]);
        let codex = run(&lib, &agent("codex", J, &["library/P.yaml"]));
        assert_eq!(
            problems(&codex.problems),
            [
                row("library/empty", "codex-skill-description-empty"),
                row("library/Upper", SKILL_NAME_INVALID),
            ]
        );
        let claude = run(&lib, &agent("claude", J, &["library/P.yaml"]));
        assert!(claude.problems.is_empty(), "{:?}", claude.problems);
        let gamma = run(&lib, &agent("gamma", J, &["library/P.yaml"]));
        assert_eq!(
            problems(&gamma.problems),
            [row("library/odd", "gamma-skill-refused")]
        );
        assert_eq!(skill_names(&gamma), ["empty", "Upper", "odd"]);
        let shown = run(&lib, &alone("library/P.yaml"));
        assert!(
            shown.problems.is_empty(),
            "no format verdict without an agent"
        );
        assert_eq!(skill_names(&shown), ["empty", "Upper", "odd"]);
    }

    // U5 (a): in one preset on its own a pair collides only if some backend would load both.
    #[test]
    fn alone_a_same_name_pair_collides_only_when_some_backend_loads_both() {
        let item = |path: &str, tag: Option<&str>| match tag {
            None => format!("  - library/{path}\n"),
            Some(tag) => format!("  - {{path: library/{path}, backend: {tag}}}\n"),
        };
        for (first, second, collides) in [
            (None, None, true),
            (None, Some("claude"), true),
            (Some("codex"), Some("codex"), true),
            (Some("claude"), Some("codex"), false),
            (Some("gamma"), Some("gamma"), true),
            (None, Some("gamma"), true),
            (Some("gamma"), Some("claude"), false),
        ] {
            let lists = format!("skills:\n{}{}", item("one", first), item("two", second));
            let lib = library(&[
                ("P.yaml", &preset(&lists)),
                ("one/SKILL.md", &skill("same", "1")),
                ("two/SKILL.md", &skill("same", "2")),
            ]);
            let resolution = run(&lib, &alone("library/P.yaml"));
            let expected = if collides {
                vec![row("library/one+library/two", SKILL_NAME_CONFLICT)]
            } else {
                Vec::new()
            };
            assert_eq!(
                problems(&resolution.problems),
                expected,
                "{first:?} × {second:?}"
            );
        }
    }

    // ── settings and tags ────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn an_agent_loads_only_its_backends_field_and_is_blocked_only_by_its_refusal() {
        let lib = library(&[
            ("P.yaml", &preset("settings:\n  - library/s.yaml\n")),
            (
                "s.yaml",
                "kind: settings\nversion: 1\nclaude:\n  model: m\ncodex:\n  - foo\ngamma:\n  unsafe: true\n",
            ),
        ]);
        let claude = run(&lib, &agent("claude", J, &["library/P.yaml"]));
        assert_eq!(
            claude.raw_settings,
            [ResolvedSettings {
                asset: AssetRef::parse("library/s.yaml").unwrap(),
                via: via(&["library/P.yaml"]),
                value: json!({ "model": "m" }),
            }]
        );
        assert!(
            claude.problems.is_empty(),
            "another backend's refusal does not block"
        );
        let codex = run(&lib, &agent("codex", T, &["library/P.yaml"]));
        assert_eq!(codex.raw_settings[0].value, json!(["foo"]));
        assert_eq!(
            problems(&codex.problems),
            [row("library/s.yaml", "codex-config-line-invalid")]
        );
        let gamma = run(&lib, &agent("gamma", J, &["library/P.yaml"]));
        assert_eq!(gamma.raw_settings[0].value, json!({ "unsafe": true }));
        assert_eq!(
            problems(&gamma.problems),
            [row("library/s.yaml", "gamma-line-unsafe")]
        );
        let shown = run(&lib, &alone("library/P.yaml"));
        assert_eq!(
            shown.raw_settings[0].value,
            json!({ "claude": { "model": "m" }, "codex": ["foo"], "gamma": { "unsafe": true } })
        );
        assert_eq!(
            problems(&shown.problems),
            [
                row("library/s.yaml", "codex-config-line-invalid"),
                row("library/s.yaml", "gamma-line-unsafe"),
            ]
        );
    }

    // D2: no notice for an item tagged for another backend or a settings asset without this backend's field.
    #[test]
    fn another_backends_items_and_fields_are_skipped_without_a_notice() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - {path: library/k.md, backend: codex}\nskills:\n  - {path: library/s, backend: codex}\nsettings:\n  - library/c.yaml\n"),
            ),
            ("k.md", "k"),
            ("s/SKILL.md", &skill("s", "d")),
            ("c.yaml", "kind: settings\nversion: 1\ncodex:\n  - model=o3\n"),
        ]);
        for mode in [T, J] {
            let claude = run(&lib, &agent("claude", mode, &["library/P.yaml"]));
            assert_eq!(claude, Resolution::default(), "{mode:?}");
        }
    }

    #[test]
    fn the_terminal_mode_names_what_it_leaves_out_in_its_backends_notice() {
        let dir = full();
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - library/N.yaml\ninstructions:\n  - library/a.md\n  - {path: library/k.md, backend: codex}\n  - {path: library/l.md, backend: claude}\nskills:\n  - library/s\n"),
            ),
            (
                "N.yaml",
                &preset("instructions:\n  - library/n.md\nskills:\n  - library/t\n"),
            ),
            ("a.md", "a"),
            ("k.md", "k"),
            ("l.md", "l"),
            ("n.md", "n"),
            ("s/SKILL.md", &skill("s", "d")),
            ("t/SKILL.md", &skill("t", "d")),
        ]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("codex", T, &["library/P.yaml"]);
        target.default_preset = true;
        let terminal = resolve_with(&lib, &builtins, &target);
        assert!(terminal.include_priming, "the priming still loads");
        assert!(terminal.instructions.is_empty() && terminal.skills.is_empty());
        assert_eq!(
            terminal.notices,
            [reason(
                "codex-terminal-omitted",
                json!({ "instructions": ["library/n.md", "library/a.md", "library/k.md"], "skills": ["t", "s"] }),
                None
            )]
        );
        target.backend = Some("claude".to_owned());
        let claude = resolve_with(&lib, &builtins, &target);
        assert_eq!(
            instructions(&claude),
            ["library/n.md", "library/a.md", "library/l.md"]
        );
        assert!(claude.notices.is_empty());
    }

    #[test]
    fn a_fake_third_backend_selects_its_tags_and_fields_through_the_registry() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - library/c.md\n  - {path: library/g.md, backend: GAMMA}\n  - {path: library/k.md, backend: claude}\nsettings:\n  - library/s.yaml\n"),
            ),
            ("c.md", "c"),
            ("g.md", "g"),
            ("k.md", "k"),
            (
                "s.yaml",
                "kind: settings\nversion: 1\nclaude: {a: 1}\nGamma: {b: 2}\n",
            ),
        ]);
        let gamma = run(&lib, &agent("gamma", J, &["library/P.yaml"]));
        assert_eq!(instructions(&gamma), ["library/c.md", "library/g.md"]);
        assert_eq!(gamma.raw_settings.len(), 1);
        assert_eq!(gamma.raw_settings[0].value, json!({ "b": 2 }));
        assert!(gamma.problems.is_empty() && gamma.notices.is_empty());
    }

    // ── folder entries ───────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn folder_entries_keep_their_first_place_by_normalized_spelling_and_tell_nothing() {
        let rules = absolute("rules");
        let again = absolute("x/../rules/");
        let other = absolute("other");
        let lib = library(&[
            (
                "P.yaml",
                &preset(&format!(
                    "presets:\n  - library/N.yaml\nfolders:\n  - '{again}'\n  - '{other}'\n"
                )),
            ),
            ("N.yaml", &preset(&format!("folders:\n  - '{rules}'\n"))),
        ]);
        let resolution = run(&lib, &agent("codex", J, &["library/P.yaml"]));
        assert_eq!(
            resolution.folder_paths(),
            [normalized(&rules), normalized(&other)]
        );
        assert_eq!(
            resolution.folders[0].via,
            via(&["library/P.yaml", "library/N.yaml"])
        );
        assert_eq!(resolution.folders[0].key, path_key(&normalized(&rules)));
        assert_eq!(resolution.folders[0].status, None);
        assert!(resolution.notices.is_empty() && resolution.problems.is_empty());
    }

    /// An absent folder (missing, not a folder) is a ① `folder-not-found` row with `{path}`; one that exists but
    /// cannot be read or did not answer is a ② `folder-unreadable` row with `{path, why}` — on every backend and mode
    /// (ADR-0387).
    #[test]
    fn a_folder_that_is_not_ok_is_a_missing_or_unreadable_row_on_every_backend_and_mode() {
        let folder = absolute("f");
        let lib = library(&[("P.yaml", &preset(&format!("folders:\n  - '{folder}'\n")))]);
        let path = normalized(&folder);
        let shown = path.to_string_lossy().into_owned();
        let mut targets = vec![alone("library/P.yaml")];
        for word in ["claude", "codex", "gamma"] {
            for mode in [T, J] {
                targets.push(agent(word, mode, &["library/P.yaml"]));
            }
        }
        let item = format!("folder:{shown}");
        let missing = BlockReason::Missing {
            item: item.clone(),
            via: via(&["library/P.yaml"]),
            reason: reason(FOLDER_NOT_FOUND, json!({ "path": shown }), Some(4)),
        };
        let unreadable = |why: &str| BlockReason::Broken {
            item: item.clone(),
            via: via(&["library/P.yaml"]),
            reason: reason(
                FOLDER_UNREADABLE,
                json!({ "path": shown, "why": why }),
                Some(4),
            ),
        };
        for target in &targets {
            for (status, row) in [
                (FolderStatus::Missing, missing.clone()),
                (FolderStatus::NotADirectory, missing.clone()),
                (
                    FolderStatus::Unreadable("timeout".to_owned()),
                    unreadable("timeout"),
                ),
                (
                    FolderStatus::Unreadable(crate::folder::SHARE_NOT_REACHABLE.to_owned()),
                    unreadable(crate::folder::SHARE_NOT_REACHABLE),
                ),
            ] {
                let mut resolution = run(&lib, target);
                resolution
                    .attach_folder_status(&HashMap::from([(path_key(&path), status.clone())]));
                assert_eq!(resolution.folders[0].status, Some(status));
                assert_eq!(resolution.problems, [row], "{target:?}");
            }
            let mut healthy = run(&lib, target);
            healthy.attach_folder_status(&HashMap::from([(path_key(&path), FolderStatus::Ok)]));
            assert_eq!(healthy.folders[0].status, Some(FolderStatus::Ok));
            assert!(healthy.problems.is_empty());
        }
    }

    #[test]
    fn a_folder_the_agents_command_line_cannot_carry_is_broken_for_that_backend_only() {
        let folder = absolute("a%b");
        let lib = library(&[("P.yaml", &preset(&format!("folders:\n  - '{folder}'\n")))]);
        let shown = normalized(&folder).to_string_lossy().into_owned();
        let claude = run(&lib, &agent("claude", T, &["library/P.yaml"]));
        assert_eq!(
            claude.problems,
            [BlockReason::Broken {
                item: format!("folder:{shown}"),
                via: via(&["library/P.yaml"]),
                reason: reason(FOLDER_PATH_CLI_UNSAFE, json!({ "path": shown }), Some(4)),
            }]
        );
        assert_eq!(claude.folders.len(), 1, "still a folder entry");
        assert!(run(&lib, &agent("codex", J, &["library/P.yaml"]))
            .problems
            .is_empty());
        assert!(run(&lib, &alone("library/P.yaml")).problems.is_empty());
    }

    // ── block ④ and the block ────────────────────────────────────────────────────────────────────────────

    // ADR-0400 decision 19: block ④ stands per mount — the items reached in a mount that cannot be used are one row
    //   naming it (none of them looked up), and every other mount, the built-ins and the attached spellings resolve
    //   as they would. A launch sees an absent mount's items as ① rows instead.
    #[test]
    fn a_mount_that_cannot_be_used_is_one_block_four_row_naming_it_and_other_mounts_expand() {
        let lib = library(&[
            (
                "Q.yaml",
                &preset(
                    "instructions:\n  - library/ok.md\n  - team/a.md\n  - team/b.md\n  - old/c.md\n  - library/gone.md\n",
                ),
            ),
            ("ok.md", "ok"),
        ]);
        let raw = "../out.yaml";
        let unparsed = vec![(raw.to_owned(), AssetRef::parse(raw).unwrap_err())];
        let mut target = agent("claude", J, &["library/Q.yaml", "team/T.yaml"]);
        target.default_preset = true;
        target.unparsed = unparsed;
        let not_registered = block_four("old", NOT_REGISTERED);
        for (team, row_of_team, absent) in [
            (
                RootState::Unreadable("access denied".to_owned()),
                block_four("team", "access denied"),
                false,
            ),
            (
                RootState::Absent("missing".to_owned()),
                block_four("team", "missing"),
                true,
            ),
        ] {
            target.mounts = states(None, Some(team.clone()), false);
            let resolution = run(&lib, &target);
            assert_eq!(instructions(&resolution), ["library/ok.md"], "{team:?}");
            assert_eq!(resolution.notices.len(), 1, "the default preset's stand-in");
            assert_eq!(
                resolution.problems,
                [
                    row_of_team.clone(),
                    not_registered.clone(),
                    BlockReason::Missing {
                        item: "library/gone.md".to_owned(),
                        via: via(&["library/Q.yaml"]),
                        reason: reason(NOT_FOUND, json!({ "path": "library/gone.md" }), Some(8)),
                    },
                    BlockReason::Broken {
                        item: raw.to_owned(),
                        via: Vec::new(),
                        reason: reason(
                            ITEM_PATH_INVALID,
                            json!({ "path": raw, "why": PathError::DotName.to_string() }),
                            None,
                        ),
                    },
                ],
                "{team:?}: one row for team (its attached preset and both items), one for the name no mount has"
            );
            let mut absent_mounts = vec![("old".to_owned(), NOT_REGISTERED.to_owned())];
            if absent {
                absent_mounts.insert(0, ("team".to_owned(), "missing".to_owned()));
            }
            assert_eq!(resolution.absent_mounts, absent_mounts, "{team:?}");
            // ADR-0387: an absent mount's ④ row is skipped; an unreadable one's blocks.
            let blocked = resolution.blocked().expect("the unparsed spelling blocks");
            assert_eq!(blocked.reasons.contains(&row_of_team), !absent, "{team:?}");
            assert!(!blocked.reasons.contains(&not_registered));
        }

        // A launch: an absent mount's items are ① rows, its folder named once in `absent_mounts`; an unreadable
        //   mount's ④ row is the same.
        target.mounts = states(None, Some(RootState::Absent("missing".to_owned())), true);
        let launch = run(&lib, &target);
        let missing = |item: &str, via_q: bool, line: Option<u32>| BlockReason::Missing {
            item: item.to_owned(),
            via: if via_q {
                via(&["library/Q.yaml"])
            } else {
                Vec::new()
            },
            reason: reason(NOT_FOUND, json!({ "path": item }), line),
        };
        assert_eq!(
            launch.problems[..5],
            [
                missing("team/a.md", true, Some(5)),
                missing("team/b.md", true, Some(6)),
                missing("old/c.md", true, Some(7)),
                missing("library/gone.md", true, Some(8)),
                missing("team/T.yaml", false, None),
            ][..]
        );
        assert_eq!(
            launch.absent_mounts,
            [
                ("team".to_owned(), "missing".to_owned()),
                ("old".to_owned(), NOT_REGISTERED.to_owned())
            ]
        );
        target.mounts = states(None, Some(RootState::Unreadable("denied".to_owned())), true);
        assert_eq!(run(&lib, &target).problems[0], block_four("team", "denied"));

        // With its own mount unusable, the attached library preset is that one row; the built-ins still resolve.
        let mut only = agent("claude", J, &["library/Q.yaml"]);
        only.default_preset = true;
        only.mounts = states(Some(RootState::Unreadable("x".to_owned())), None, false);
        let resolution = run(&lib, &only);
        assert_eq!(resolution.problems, [block_four("library", "x")]);
        assert!(resolution.instructions.is_empty() && resolution.notices.len() == 1);
    }

    // A usable mount shows its ① ② ③ rows; with no mount registered every library preset is one skipped ④ row for its
    //   first name (ADR-0400 decision 19 — not registered is absent).
    #[test]
    fn with_no_mount_registered_a_library_preset_is_one_skipped_block_four_row() {
        let folder = absolute("f");
        let lib = library(&[
            ("P.yaml", &preset("")),
            (
                "Q.yaml",
                &preset(&format!(
                    "instructions:\n  - library/x.yaml\n  - library/gone.md\n  - library/ok.md\nskills:\n  - library/a\n  - library/b\nfolders:\n  - '{folder}'\n"
                )),
            ),
            ("x.yaml", "kind: preset\nversion: [\n"),
            ("ok.md", "ok"),
            ("a/SKILL.md", &skill("s", "1")),
            ("b/SKILL.md", &skill("s", "2")),
        ]);
        let raw = "../out.yaml";
        let unparsed = vec![(raw.to_owned(), AssetRef::parse(raw).unwrap_err())];
        // With its mount usable, Q loads items, a folder and the default preset's notice, and shows ① ② ③ rows; the
        //   attached spelling that is not a reference is a ② row too.
        let mut usable = agent("claude", J, &["library/Q.yaml"]);
        usable.default_preset = true;
        usable.unparsed = unparsed.clone();
        let shown = run(&lib, &usable);
        assert_eq!(
            problems(&shown.problems),
            [
                row("library/x.yaml", "yaml-syntax"),
                row("library/gone.md", NOT_FOUND),
                row(raw, ITEM_PATH_INVALID),
                row("library/a+library/b", SKILL_NAME_CONFLICT),
            ],
            "the rows of a usable mount"
        );
        assert_eq!(instructions(&shown), ["library/ok.md"]);
        assert_eq!(skill_names(&shown), ["s", "s"]);
        assert_eq!(shown.folders.len(), 1);
        assert_eq!(shown.notices.len(), 1, "the default preset's stand-in");
        // With no mount registered, every library preset is one ④ row for its first name, skipped; the built-ins
        //   alone resolve.
        let mut stale = agent("claude", J, &["library/Q.yaml", "library/P.yaml"]);
        stale.mounts = MountStates::new(&Mounts::default(), false);
        let resolution = run(&lib, &stale);
        assert_eq!(resolution.problems, [block_four("library", NOT_REGISTERED)]);
        assert_eq!(
            (resolution.skipped().len(), resolution.blocked()),
            (1, None)
        );
        let mut only_builtin = agent("claude", J, &[]);
        only_builtin.default_preset = true;
        only_builtin.mounts = MountStates::new(&Mounts::default(), false);
        assert!(
            run(&lib, &only_builtin).problems.is_empty(),
            "no library item reached"
        );
    }

    /// One rule for a registered mount (ADR-0387): a stat that says missing or not a folder is absent whatever the first scan
    /// did; a failed first scan or an unreadable stat is unreadable; no stat and a good scan is `Ok`.
    #[test]
    fn the_root_state_rule_tells_absent_from_unreadable() {
        let failed: Result<(), String> = Err("scan failed".to_owned());
        let unreadable = FolderStatus::Unreadable("denied".to_owned());
        for (joined, stat, state) in [
            (Ok(()), Some(FolderStatus::Ok), RootState::Ok),
            (Ok(()), None, RootState::Ok),
            (
                Ok(()),
                Some(FolderStatus::Missing),
                RootState::Absent("missing".to_owned()),
            ),
            (
                failed.clone(),
                Some(FolderStatus::NotADirectory),
                RootState::Absent("not a folder".to_owned()),
            ),
            (
                Ok(()),
                Some(unreadable.clone()),
                RootState::Unreadable("denied".to_owned()),
            ),
            (
                failed.clone(),
                Some(unreadable),
                RootState::Unreadable("scan failed".to_owned()),
            ),
            (
                failed.clone(),
                Some(FolderStatus::Ok),
                RootState::Unreadable("scan failed".to_owned()),
            ),
            (
                failed,
                None,
                RootState::Unreadable("scan failed".to_owned()),
            ),
        ] {
            assert_eq!(
                RootState::of(&joined, stat.as_ref()),
                state,
                "{joined:?} {stat:?}"
            );
        }
    }

    // ADR-0400 decision 19: a library item the built-in default lists is judged by its mount like any other — not found
    //   in a usable mount, one ④ row for a mount that cannot be used.
    #[test]
    fn a_library_item_the_builtin_default_lists_is_judged_by_its_mount() {
        let dir = crate::builtin::tests::folder(&[(
            crate::builtin::file_of(BuiltinId::DefaultPreset),
            "kind: preset\nversion: 1\ninstructions:\n  - library/x.md\n",
        )]);
        let lib = library(&[]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("claude", J, &[]);
        target.default_preset = true;
        let resolution = resolve_with(&lib, &builtins, &target);
        assert_eq!(
            resolution.problems,
            [BlockReason::Missing {
                item: "library/x.md".to_owned(),
                via: via(&["engram/default"]),
                reason: reason(NOT_FOUND, json!({ "path": "library/x.md" }), Some(4)),
            }]
        );
        target.mounts = states(Some(RootState::Absent("missing".to_owned())), None, false);
        let resolution = resolve_with(&lib, &builtins, &target);
        assert_eq!(resolution.problems, [block_four("library", "missing")]);
        assert_eq!(resolution.blocked(), None);
    }

    // TRD S25 §1-3 8: the block is the problem list less the skipped rows — the same rows whoever launches, ordered ②
    //   ③ ④ (block ④ per mount — a_mount_that_cannot_be_used_is_one_block_four_row_naming_it_and_other_mounts_expand).
    //   ADR-0387: the ① rows are skipped, an unreadable folder blocks; the two parts are exactly the problems.
    #[test]
    fn the_block_is_the_problem_list_less_the_skipped_rows_ordered_by_kind() {
        let (first, second) = (absolute("z-gone"), absolute("a-gone"));
        let lib = library(&[
            (
                "P.yaml",
                &preset(&format!(
                    "instructions:\n  - library/x.yaml\nskills:\n  - library/a\n  - library/b\nfolders:\n  - '{first}'\n  - '{second}'\n"
                )),
            ),
            ("x.yaml", "kind: preset\nversion: [\n"),
            ("a/SKILL.md", &skill("s", "1")),
            ("b/SKILL.md", &skill("s", "2")),
        ]);
        let mut resolution = run(&lib, &agent("claude", J, &["library/P.yaml"]));
        let (first, second) = (normalized(&first), normalized(&second));
        resolution.attach_folder_status(&HashMap::from([
            (path_key(&first), FolderStatus::Missing),
            (
                path_key(&second),
                FolderStatus::Unreadable("denied".to_owned()),
            ),
        ]));
        let blocked = resolution.blocked().expect("blocked");
        assert_eq!(
            problems(&blocked.reasons),
            [
                row("library/x.yaml", "yaml-syntax"),
                row(
                    &format!("folder:{}", second.to_string_lossy()),
                    FOLDER_UNREADABLE
                ),
                row("library/a+library/b", SKILL_NAME_CONFLICT),
            ],
            "by kind, expansion order kept within a kind"
        );
        assert_eq!(
            problems(&resolution.skipped()),
            [row(
                &format!("folder:{}", first.to_string_lossy()),
                FOLDER_NOT_FOUND
            )],
            "the absent folder is skipped"
        );
        let mut rows = blocked.reasons.clone();
        rows.extend(resolution.skipped());
        rows.sort();
        let mut found = resolution.problems.clone();
        found.sort();
        assert_eq!(
            rows, found,
            "the two parts are every problem row and nothing else"
        );
        assert_eq!(run(&lib, &agent("claude", J, &[])).blocked(), None);

        // Only skipped rows: nothing blocks.
        let only_missing = library(&[("M.yaml", &preset("instructions:\n  - library/gone.md\n"))]);
        let skipped = run(
            &only_missing,
            &agent("claude", J, &["library/M.yaml", "library/absent.yaml"]),
        );
        assert_eq!(skipped.blocked(), None);
        assert_eq!(skipped.skipped(), skipped.problems);
        assert_eq!(skipped.problems.len(), 2);
    }

    // ── review round 1 ───────────────────────────────────────────────────────────────────────────────────

    // U5 (a) with a repeated path: s1 is first reached tagged for claude, then again as common — a codex agent loads
    //   s1 and s2, so the preset on its own shows their conflict too.
    #[test]
    fn alone_a_skill_listed_again_keeps_every_backend_it_is_listed_for() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - library/A.yaml\n  - library/B.yaml\n"),
            ),
            (
                "A.yaml",
                &preset("skills:\n  - {path: library/s1, backend: claude}\n"),
            ),
            (
                "B.yaml",
                &preset("skills:\n  - library/s1\n  - {path: library/s2, backend: codex}\n"),
            ),
            ("s1/SKILL.md", &skill("dup", "1")),
            ("s2/SKILL.md", &skill("dup", "2")),
        ]);
        let codex = run(&lib, &agent("codex", J, &["library/P.yaml"]));
        assert_eq!(
            problems(&codex.problems),
            [row("library/s1+library/s2", SKILL_NAME_CONFLICT)]
        );
        let shown = run(&lib, &alone("library/P.yaml"));
        assert_eq!(
            problems(&shown.problems),
            [row("library/s1+library/s2", SKILL_NAME_CONFLICT)]
        );
        assert_eq!(
            shown.skills[0].2,
            via(&["library/P.yaml", "library/A.yaml"]),
            "first place kept"
        );
    }

    #[test]
    fn the_same_path_in_two_lists_is_judged_in_each() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - library/a.md\nskills:\n  - library/a.md\n"),
            ),
            ("a.md", "a"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["library/P.yaml"]));
        assert_eq!(instructions(&resolution), ["library/a.md"]);
        assert_eq!(
            problems(&resolution.problems),
            [row("library/a.md", WRONG_KIND)]
        );
    }

    #[test]
    fn an_attached_spelling_that_is_not_a_reference_is_a_broken_row_with_its_text_escaped() {
        let lib = library(&[]);
        let raw = "../x\u{1b}.yaml".to_owned();
        let why = AssetRef::parse(&raw).unwrap_err();
        let mut target = agent("claude", J, &[]);
        target.unparsed = vec![(raw.clone(), why.clone())];
        let resolution = run(&lib, &target);
        assert_eq!(
            resolution.problems,
            [BlockReason::Broken {
                item: raw,
                via: Vec::new(),
                reason: reason(
                    ITEM_PATH_INVALID,
                    json!({ "path": "../x\\u{1b}.yaml", "why": why.to_string() }),
                    None
                ),
            }]
        );
        assert!(resolution.blocked().is_some());
    }

    #[test]
    fn a_fake_third_backend_drives_the_omitted_notice_the_name_check_and_the_folder_block() {
        let blocked = absolute("blocked");
        let lib = library(&[
            (
                "P.yaml",
                &preset(&format!(
                    "instructions:\n  - library/a.md\nskills:\n  - library/bad-one\n  - {{path: library/s, backend: gamma}}\nfolders:\n  - '{blocked}'\n"
                )),
            ),
            ("a.md", "a"),
            ("bad-one/SKILL.md", &skill("bad-one", "d")),
            ("s/SKILL.md", &skill("s", "d")),
        ]);
        let terminal = run(&lib, &agent("gamma", T, &["library/P.yaml"]));
        assert_eq!(
            terminal.notices,
            [reason(
                "gamma-omitted",
                json!({ "instructions": ["library/a.md"], "skills": ["bad-one", "s"] }),
                None
            )]
        );
        assert!(terminal.instructions.is_empty() && terminal.skills.is_empty());
        let shown = normalized(&blocked).to_string_lossy().into_owned();
        let json_mode = run(&lib, &agent("gamma", J, &["library/P.yaml"]));
        assert_eq!(
            problems(&json_mode.problems),
            [
                row("library/bad-one", SKILL_NAME_INVALID),
                row(&format!("folder:{shown}"), FOLDER_PATH_CLI_UNSAFE),
            ]
        );
        assert_eq!(
            problems(&terminal.problems),
            [row(&format!("folder:{shown}"), FOLDER_PATH_CLI_UNSAFE)]
        );
        assert!(json_mode.notices.is_empty());
        assert!(run(&lib, &alone("library/P.yaml")).problems.is_empty());
    }
}
