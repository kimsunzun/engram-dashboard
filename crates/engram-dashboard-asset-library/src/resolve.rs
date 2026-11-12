//! Resolution — 「실제로 받는 것」: what an agent (or one preset on its own) gets from its attached presets, its
//! problem list and its notices ([`resolve`]).
//!
//! ## Invariants
//!
//! - **Pure** — no disk, no lock, no clock. Items come through [`Lookup`] (the catalog plus the built-ins, or a launch's
//!   overlay); folder entries are only collected here, and their disk status arrives later through
//!   [`Resolution::attach_folder_status`] (phase 2 — the caller stats [`Resolution::folder_paths`] outside every lock).
//! - **Expansion** — for an agent, `engram:/default` (when attached) and then the attached presets in order; one preset =
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
//!   launch block come from the same rows.
// ADR-0315
// ADR-0341
// ADR-0359
// TRD S25 step-7 §3-7

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use engram_dashboard_agent::backend::settings_check::{InstructionLoad, ModeLoads, SkillFormat};
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::{
    escape_controls, BlockReason, LaunchBlocked, Notice, Reason, BUILTIN_UNAVAILABLE,
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
use crate::gate::{NOT_FOUND, NO_ROOT};
use crate::path::{AssetRef, BuiltinId, FolderRef, PathError, PresetItem};
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

/// The code of a folder entry that is missing, not a folder or could not be checked (`args` = `{path}`, plus `why` —
/// the OS error, [`crate::folder::TIMEOUT`], [`crate::folder::NOT_CHECKED`] (never stat'ed — the budget was spent), or
/// a failed worker ("spawn failed", "worker panicked") — when it was not found to be a folder for another reason).
// ADR-0341
// TRD S25 step-7 B8
pub const FOLDER_NOT_FOUND: &str = "folder-not-found";

/// The code of a folder entry the agent's command line cannot carry (`args` = `{path}`).
// TRD S25 step-7 B8
pub const FOLDER_PATH_CLI_UNSAFE: &str = "folder-path-cli-unsafe";

/// One problem row — the launch block row itself, so the problem list and the block are the same rows. `item` = a
/// library path, `engram:/…` or `folder:<normalized path>`; `via` = the presets from the expansion start (for an agent,
/// the attached preset) down to the one that lists the item — empty for an attached preset itself. A conflict carries
/// every skill involved with its own chain. `reason.line` is a line of the referencing preset (`via.last()`; a row
/// for an attached preset itself has `None`) for a row about a reference — `not-found`, `wrong-kind`, `preset-cycle`,
/// `builtin-in-library-preset`, `path-through-link`, a folder entry — and a line of the item's own file for a row about
/// its content when the content gives one (line problems, `yaml-syntax`, settings refusals, skill format). Rows
/// with no line: skill-name checks and conflicts, block ④, an unparsed attached path.
// The TRD's `Problem { kind, item, reason, via }` cannot carry a conflict's several items (parent §1-3 5).
pub type Problem = BlockReason;

/// What [`resolve`] resolves.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// The agent's backend word; `None` = one preset on its own (no agent — everything is shown).
    pub backend: Option<String>,
    /// The agent's mode; `None` with `backend: None`.
    pub mode: Option<AgentOutputFormat>,
    /// Whether `engram:/default` is attached (it expands first).
    pub default_preset: bool,
    /// The attached presets, in attach order.
    pub presets: Vec<AssetRef>,
    /// Attached preset spellings that are not references, with why — each is a ② row (an attached part is
    /// required — TRD S25 §1-3 8).
    pub unparsed: Vec<(String, PathError)>,
    /// Decides block ④ when a library preset is attached.
    pub root: RootState,
}

/// The library root as a computation found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootState {
    Unset,
    /// The root is set but cannot be used — why: `missing`, `not a folder`, the stat's diagnostic, or why the first
    /// scan could not be applied.
    Unavailable(String),
    Ok,
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

/// What [`resolve`] answers. A block-④ resolution holds that one problem and nothing else (every other field empty).
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
    /// In expansion order; rows for unparsed attached paths after the rows of the expansion, then conflicts, folder ①
    /// lines (phase 2) last.
    pub problems: Vec<Problem>,
    pub notices: Vec<Notice>,
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

    /// Phase 2: records each folder's status from `statuses` (by key) and adds a ① line for every folder that is not
    /// `Ok` — on every backend and mode (ADR-0341 decision 6: one that exists but cannot be read is ① too). A folder
    /// `statuses` lacks keeps `None` and gets no line.
    pub fn attach_folder_status(&mut self, statuses: &HashMap<PathKey, FolderStatus>) {
        for folder in &mut self.folders {
            let Some(status) = statuses.get(&folder.key) else {
                continue;
            };
            folder.status = Some(status.clone());
            let path = folder.path.to_string_lossy();
            let args = match status {
                FolderStatus::Ok => continue,
                FolderStatus::Missing | FolderStatus::NotADirectory => json!({ "path": path }),
                FolderStatus::Unreadable(why) => json!({ "path": path, "why": why }),
            };
            self.problems.push(BlockReason::Missing {
                item: folder_item(&folder.path),
                via: folder.via.clone(),
                reason: reason(FOLDER_NOT_FOUND, args, Some(folder.line)),
            });
        }
    }

    /// The launch block: every problem, ordered by reason kind (① ② ③ ④ — TRD S25 §1-3 8), expansion order kept within
    /// a kind. `None` = no problem.
    pub fn blocked(&self) -> Option<LaunchBlocked> {
        if self.problems.is_empty() {
            return None;
        }
        let mut reasons = self.problems.clone();
        reasons.sort_by_key(|problem| match problem {
            BlockReason::Missing { .. } => 0,
            BlockReason::Broken { .. } => 1,
            BlockReason::Conflict { .. } => 2,
            BlockReason::LibraryUnavailable { .. } => 3,
        });
        Some(LaunchBlocked { reasons })
    }
}

/// Resolves `target` over `lookup` (module header). Block ④ = a library preset attached while `target.root` is not
/// `Ok`; the resolution is then block ④ alone — no expansion, no item, no notice: while the root is unusable the
/// catalog cannot be trusted, so nothing computed from it is in the answer. The rows of unparsed attached paths (from
/// the profile) and the built-in stand-in notice go too: while the root is unusable the block is one line (user
/// decision 2026-11-12).
pub fn resolve(target: &Target, lookup: &dyn Lookup, backends: &dyn Backends) -> Resolution {
    let library_attached = target
        .presets
        .iter()
        .any(|preset| matches!(preset, AssetRef::Library(_)));
    let unavailable = match &target.root {
        RootState::Ok => None,
        RootState::Unset => Some(NO_ROOT),
        RootState::Unavailable(why) => Some(why.as_str()),
    };
    if let (true, Some(message)) = (library_attached, unavailable) {
        return Resolution {
            problems: vec![BlockReason::LibraryUnavailable {
                reason: reason(LIBRARY_UNAVAILABLE, json!({ "message": message }), None),
            }],
            ..Resolution::default()
        };
    }
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

/// The catalog and the built-ins as a [`Lookup`] — a library item is present when the catalog holds it.
pub(crate) struct CatalogLookup<'a> {
    pub(crate) catalog: &'a Catalog,
    /// The root the catalog was scanned from — library disk paths are taken under it.
    pub(crate) root: Option<&'a Path>,
    pub(crate) builtins: &'a Builtins,
}

impl Lookup for CatalogLookup<'_> {
    fn item(&self, r: &AssetRef) -> Looked<'_> {
        match r {
            AssetRef::BuiltIn(id) => Looked::Present(self.builtins.view(*id)),
            AssetRef::Library(path) => match self.catalog.get(&key(path)) {
                Some(item) => Looked::Present(ItemView {
                    kind: &item.kind,
                    content: &item.content,
                    file: self.root.map(|root| item.path.to_path(root)),
                    stand_in: None,
                }),
                None => Looked::Missing,
            },
        }
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
enum RefKey {
    Library(PathKey),
    BuiltIn(BuiltinId),
}

fn ref_key(r: &AssetRef) -> RefKey {
    match r {
        AssetRef::Library(path) => RefKey::Library(key(path)),
        AssetRef::BuiltIn(id) => RefKey::BuiltIn(*id),
    }
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
        let ItemContent::Preset { doc } = view.content else {
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
        let lookup: &'l dyn Lookup = self.lookup;
        match lookup.item(at) {
            Looked::Missing => {
                let args = json!({ "path": spelling });
                self.out.problems.push(BlockReason::Missing {
                    item: spelling,
                    via: via.to_vec(),
                    reason: reason(NOT_FOUND, args, line),
                });
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

    /// Whether a tag or raw field named `word` belongs to this resolution: one preset on its own takes every word; an
    /// agent only its backend's.
    fn takes_backend(&self, word: &str) -> bool {
        match self.agent {
            None => true,
            Some(agent) => self
                .backends
                .for_word(word)
                .is_some_and(|check| check.backend() == agent.check.backend()),
        }
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

/// The `item` of a folder entry's problem.
fn folder_item(path: &Path) -> String {
    format!("folder:{}", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::tests::full;
    use crate::catalog::harness::Lib;
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
            root: RootState::Ok,
        }
    }

    fn alone(preset: &str) -> Target {
        Target {
            backend: None,
            mode: None,
            default_preset: false,
            presets: refs(&[preset]),
            unparsed: Vec::new(),
            root: RootState::Ok,
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
        let lookup = CatalogLookup {
            catalog: &lib.catalog,
            root: Some(&lib.root),
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
                &preset("presets:\n  - C.yaml\ninstructions:\n  - a.md\n"),
            ),
            (
                "B.yaml",
                &preset("presets:\n  - C.yaml\ninstructions:\n  - x.md\n  - b.md\n"),
            ),
            ("C.yaml", &preset("instructions:\n  - x.md\n  - c.md\n")),
            ("a.md", "a"),
            ("b.md", "b"),
            ("c.md", "c"),
            ("x.md", "x"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["A.yaml", "B.yaml"]));
        assert_eq!(instructions(&resolution), ["x.md", "c.md", "a.md", "b.md"]);
        assert_eq!(
            resolution.instructions[0].1,
            Path::new("/lib").join("x.md"),
            "the disk path under the root"
        );
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
    }

    // Parent §1-3 1 (G51): [A, B], A = [common a1 · codex a2], B = [common b1 · codex b2]; A holds C.
    #[test]
    fn tagged_items_follow_their_own_presets_common_items() {
        let lib = library(&[
            (
                "A.yaml",
                &preset("presets:\n  - C.yaml\ninstructions:\n  - a1.md\n  - {path: a2.md, backend: codex}\n"),
            ),
            (
                "B.yaml",
                &preset("instructions:\n  - {path: b2.md, backend: codex}\n  - b1.md\n"),
            ),
            (
                "C.yaml",
                &preset("instructions:\n  - {path: c2.md, backend: Codex}\n  - c1.md\n"),
            ),
            ("a1.md", "1"),
            ("a2.md", "2"),
            ("b1.md", "1"),
            ("b2.md", "2"),
            ("c1.md", "1"),
            ("c2.md", "2"),
        ]);
        let codex = run(&lib, &agent("codex", J, &["A.yaml", "B.yaml"]));
        assert_eq!(
            instructions(&codex),
            ["c1.md", "c2.md", "a1.md", "a2.md", "b1.md", "b2.md"]
        );
        let claude = run(&lib, &agent("claude", J, &["A.yaml", "B.yaml"]));
        assert_eq!(instructions(&claude), ["c1.md", "a1.md", "b1.md"]);
    }

    #[test]
    fn a_preset_reached_again_while_it_expands_is_a_cycle_on_that_preset() {
        let lib = library(&[
            (
                "A.yaml",
                &preset("presets:\n  - B.yaml\ninstructions:\n  - a.md\n"),
            ),
            (
                "B.yaml",
                &preset("instructions:\n  - b.md\npresets:\n  - A.yaml\n"),
            ),
            ("a.md", "a"),
            ("b.md", "b"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["A.yaml"]));
        assert_eq!(instructions(&resolution), ["b.md", "a.md"], "each once");
        assert_eq!(
            resolution.problems,
            [BlockReason::Broken {
                item: "A.yaml".to_owned(),
                via: via(&["A.yaml", "B.yaml"]),
                reason: reason(PRESET_CYCLE, json!({ "path": "A.yaml" }), Some(6)),
            }]
        );
    }

    #[test]
    fn missing_broken_wrong_kind_and_line_problems_are_rows_with_their_chain() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - N.yaml\n  - {path: Q.yaml, backend: codex}\n  - bad.yaml\n"),
            ),
            (
                "N.yaml",
                &preset("instructions:\n  - gone.md\nskills:\n  - a.md\n"),
            ),
            ("Q.yaml", &preset("")),
            ("bad.yaml", "kind: preset\nversion: [\n"),
            ("a.md", "a"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["P.yaml", "absent.yaml"]));
        assert_eq!(
            problems(&resolution.problems),
            [
                row("P.yaml", "backend-tag-not-allowed"),
                row("gone.md", NOT_FOUND),
                row("a.md", WRONG_KIND),
                row("bad.yaml", "yaml-syntax"),
                row("absent.yaml", NOT_FOUND),
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
        assert_eq!(*chain, via(&["P.yaml", "N.yaml"]));
        assert_eq!(why.args, json!({ "path": "gone.md" }));
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
        let lib = library(&[("P.yaml", &preset("presets:\n  - engram:/default\n"))]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let resolution = resolve_with(&lib, &builtins, &agent("claude", J, &["P.yaml"]));
        assert_eq!(
            resolution.problems,
            [BlockReason::Broken {
                item: "engram:/default".to_owned(),
                via: via(&["P.yaml"]),
                reason: reason(
                    BUILTIN_IN_LIBRARY_PRESET,
                    json!({ "path": "engram:/default" }),
                    Some(4)
                ),
            }]
        );
        assert!(!resolution.include_priming);
        let attached = resolve_with(&lib, &builtins, &agent("claude", J, &["engram:/default"]));
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
                via: via(&["engram:/default"]),
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
        let lib = library(&[("P.yaml", &preset("instructions:\n  - engram:/priming\n"))]);
        let mut target = agent("claude", J, &["P.yaml"]);
        target.default_preset = true;
        let resolution = run(&lib, &target);
        assert!(!resolution.include_priming);
        assert!(resolution.problems.is_empty(), "{:?}", resolution.problems);
        assert!(resolution.raw_settings.is_empty() && resolution.instructions.is_empty());
        let told = |path: &str| {
            let args = json!({ "path": path, "why": "기본 제공 폴더 없음" });
            reason(BUILTIN_UNAVAILABLE, args, None)
        };
        assert_eq!(
            resolution.notices,
            [told("engram:/default"), told("engram:/priming")],
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
        assert_eq!(notice.args["path"], "engram:/default");
        let why = notice.args["why"].as_str().unwrap();
        assert!(why.starts_with("yaml-syntax: "), "{why}");
    }

    #[test]
    fn a_stand_in_is_told_once_when_reached_and_not_at_all_when_unreached() {
        let dir = crate::builtin::tests::folder(&[
            (
                crate::builtin::file_of(BuiltinId::DefaultPreset),
                "kind: preset\nversion: 1\ninstructions:\n  - engram:/priming\n",
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
            &preset("settings:\n  - engram:/settings/default\ninstructions:\n  - engram:/settings/default\n"),
        )]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut unreached = agent("claude", J, &[]);
        unreached.default_preset = true;
        let resolution = resolve_with(&lib, &builtins, &unreached);
        assert!(resolution.include_priming);
        assert!(resolution.notices.is_empty() && resolution.problems.is_empty());
        let twice = resolve_with(&lib, &builtins, &agent("claude", J, &["P.yaml"]));
        let told: Vec<&Value> = twice.notices.iter().map(|n| &n.args["path"]).collect();
        assert_eq!(told, ["engram:/settings/default"], "reached from two lists");
        assert_eq!(
            problems(&twice.problems),
            [row("engram:/settings/default", WRONG_KIND)],
            "the stand-in is a settings item, not an instruction"
        );
    }

    // ── skills: conflicts, names and formats ─────────────────────────────────────────────────────────────

    #[test]
    fn conflicts_count_only_the_skills_the_agent_loads_and_carry_every_item_with_its_chain() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - N.yaml\nskills:\n  - s1\n  - {path: s3, backend: codex}\n  - {path: s4, backend: claude}\n"),
            ),
            ("N.yaml", &preset("skills:\n  - s2\n")),
            ("s1/SKILL.md", &skill("dup", "one")),
            ("s2/SKILL.md", &skill("Dup", "two")),
            ("s3/SKILL.md", &skill("dup", "three")),
            ("s4/SKILL.md", &skill("solo", "four")),
        ]);
        let claude = run(&lib, &agent("claude", T, &["P.yaml"]));
        assert_eq!(skill_names(&claude), ["Dup", "dup", "solo"]);
        assert_eq!(
            claude.problems,
            [BlockReason::Conflict {
                reason: reason(SKILL_NAME_CONFLICT, json!({ "name": "dup" }), None),
                items: vec![
                    ("s2".to_owned(), via(&["P.yaml", "N.yaml"])),
                    ("s1".to_owned(), via(&["P.yaml"])),
                ],
            }],
            "the codex-tagged s3 is not loaded"
        );
        let codex_json = run(&lib, &agent("codex", J, &["P.yaml"]));
        let conflicts: Vec<_> = problems(&codex_json.problems)
            .into_iter()
            .filter(|(_, code)| code == SKILL_NAME_CONFLICT)
            .collect();
        assert_eq!(conflicts, [row("s2+s1+s3", SKILL_NAME_CONFLICT)]);
        let codex_terminal = run(&lib, &agent("codex", T, &["P.yaml"]));
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
                &preset("skills:\n  - empty\n  - Upper\n  - {path: odd, backend: gamma}\n"),
            ),
            (
                "empty/SKILL.md",
                "---\nname: empty\ndescription: \"\"\n---\n",
            ),
            ("Upper/SKILL.md", &skill("Upper", "fine")),
            ("odd/SKILL.md", &skill("odd", "please refuse me")),
        ]);
        let codex = run(&lib, &agent("codex", J, &["P.yaml"]));
        assert_eq!(
            problems(&codex.problems),
            [
                row("empty", "codex-skill-description-empty"),
                row("Upper", SKILL_NAME_INVALID),
            ]
        );
        let claude = run(&lib, &agent("claude", J, &["P.yaml"]));
        assert!(claude.problems.is_empty(), "{:?}", claude.problems);
        let gamma = run(&lib, &agent("gamma", J, &["P.yaml"]));
        assert_eq!(
            problems(&gamma.problems),
            [row("odd", "gamma-skill-refused")]
        );
        assert_eq!(skill_names(&gamma), ["empty", "Upper", "odd"]);
        let shown = run(&lib, &alone("P.yaml"));
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
            None => format!("  - {path}\n"),
            Some(tag) => format!("  - {{path: {path}, backend: {tag}}}\n"),
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
            let resolution = run(&lib, &alone("P.yaml"));
            let expected = if collides {
                vec![row("one+two", SKILL_NAME_CONFLICT)]
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
            ("P.yaml", &preset("settings:\n  - s.yaml\n")),
            (
                "s.yaml",
                "kind: settings\nversion: 1\nclaude:\n  model: m\ncodex:\n  - foo\ngamma:\n  unsafe: true\n",
            ),
        ]);
        let claude = run(&lib, &agent("claude", J, &["P.yaml"]));
        assert_eq!(
            claude.raw_settings,
            [ResolvedSettings {
                asset: AssetRef::parse("s.yaml").unwrap(),
                via: via(&["P.yaml"]),
                value: json!({ "model": "m" }),
            }]
        );
        assert!(
            claude.problems.is_empty(),
            "another backend's refusal does not block"
        );
        let codex = run(&lib, &agent("codex", T, &["P.yaml"]));
        assert_eq!(codex.raw_settings[0].value, json!(["foo"]));
        assert_eq!(
            problems(&codex.problems),
            [row("s.yaml", "codex-config-line-invalid")]
        );
        let gamma = run(&lib, &agent("gamma", J, &["P.yaml"]));
        assert_eq!(gamma.raw_settings[0].value, json!({ "unsafe": true }));
        assert_eq!(
            problems(&gamma.problems),
            [row("s.yaml", "gamma-line-unsafe")]
        );
        let shown = run(&lib, &alone("P.yaml"));
        assert_eq!(
            shown.raw_settings[0].value,
            json!({ "claude": { "model": "m" }, "codex": ["foo"], "gamma": { "unsafe": true } })
        );
        assert_eq!(
            problems(&shown.problems),
            [
                row("s.yaml", "codex-config-line-invalid"),
                row("s.yaml", "gamma-line-unsafe"),
            ]
        );
    }

    // D2: no notice for an item tagged for another backend or a settings asset without this backend's field.
    #[test]
    fn another_backends_items_and_fields_are_skipped_without_a_notice() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - {path: k.md, backend: codex}\nskills:\n  - {path: s, backend: codex}\nsettings:\n  - c.yaml\n"),
            ),
            ("k.md", "k"),
            ("s/SKILL.md", &skill("s", "d")),
            ("c.yaml", "kind: settings\nversion: 1\ncodex:\n  - model=o3\n"),
        ]);
        for mode in [T, J] {
            let claude = run(&lib, &agent("claude", mode, &["P.yaml"]));
            assert_eq!(claude, Resolution::default(), "{mode:?}");
        }
    }

    #[test]
    fn the_terminal_mode_names_what_it_leaves_out_in_its_backends_notice() {
        let dir = full();
        let lib = library(&[
            (
                "P.yaml",
                &preset("presets:\n  - N.yaml\ninstructions:\n  - a.md\n  - {path: k.md, backend: codex}\n  - {path: l.md, backend: claude}\nskills:\n  - s\n"),
            ),
            (
                "N.yaml",
                &preset("instructions:\n  - n.md\nskills:\n  - t\n"),
            ),
            ("a.md", "a"),
            ("k.md", "k"),
            ("l.md", "l"),
            ("n.md", "n"),
            ("s/SKILL.md", &skill("s", "d")),
            ("t/SKILL.md", &skill("t", "d")),
        ]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("codex", T, &["P.yaml"]);
        target.default_preset = true;
        let terminal = resolve_with(&lib, &builtins, &target);
        assert!(terminal.include_priming, "the priming still loads");
        assert!(terminal.instructions.is_empty() && terminal.skills.is_empty());
        assert_eq!(
            terminal.notices,
            [reason(
                "codex-terminal-omitted",
                json!({ "instructions": ["n.md", "a.md", "k.md"], "skills": ["t", "s"] }),
                None
            )]
        );
        target.backend = Some("claude".to_owned());
        let claude = resolve_with(&lib, &builtins, &target);
        assert_eq!(instructions(&claude), ["n.md", "a.md", "l.md"]);
        assert!(claude.notices.is_empty());
    }

    #[test]
    fn a_fake_third_backend_selects_its_tags_and_fields_through_the_registry() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - c.md\n  - {path: g.md, backend: GAMMA}\n  - {path: k.md, backend: claude}\nsettings:\n  - s.yaml\n"),
            ),
            ("c.md", "c"),
            ("g.md", "g"),
            ("k.md", "k"),
            (
                "s.yaml",
                "kind: settings\nversion: 1\nclaude: {a: 1}\nGamma: {b: 2}\n",
            ),
        ]);
        let gamma = run(&lib, &agent("gamma", J, &["P.yaml"]));
        assert_eq!(instructions(&gamma), ["c.md", "g.md"]);
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
                    "presets:\n  - N.yaml\nfolders:\n  - '{again}'\n  - '{other}'\n"
                )),
            ),
            ("N.yaml", &preset(&format!("folders:\n  - '{rules}'\n"))),
        ]);
        let resolution = run(&lib, &agent("codex", J, &["P.yaml"]));
        assert_eq!(
            resolution.folder_paths(),
            [normalized(&rules), normalized(&other)]
        );
        assert_eq!(resolution.folders[0].via, via(&["P.yaml", "N.yaml"]));
        assert_eq!(resolution.folders[0].key, path_key(&normalized(&rules)));
        assert_eq!(resolution.folders[0].status, None);
        assert!(resolution.notices.is_empty() && resolution.problems.is_empty());
    }

    #[test]
    fn a_folder_that_is_not_ok_is_a_missing_line_on_every_backend_and_mode() {
        let folder = absolute("f");
        let lib = library(&[("P.yaml", &preset(&format!("folders:\n  - '{folder}'\n")))]);
        let path = normalized(&folder);
        let shown = path.to_string_lossy().into_owned();
        let mut targets = vec![alone("P.yaml")];
        for word in ["claude", "codex", "gamma"] {
            for mode in [T, J] {
                targets.push(agent(word, mode, &["P.yaml"]));
            }
        }
        for target in &targets {
            for (status, args) in [
                (FolderStatus::Missing, json!({ "path": shown })),
                (FolderStatus::NotADirectory, json!({ "path": shown })),
                (
                    FolderStatus::Unreadable("timeout".to_owned()),
                    json!({ "path": shown, "why": "timeout" }),
                ),
            ] {
                let mut resolution = run(&lib, target);
                resolution
                    .attach_folder_status(&HashMap::from([(path_key(&path), status.clone())]));
                assert_eq!(resolution.folders[0].status, Some(status));
                assert_eq!(
                    resolution.problems,
                    [BlockReason::Missing {
                        item: format!("folder:{shown}"),
                        via: via(&["P.yaml"]),
                        reason: reason(FOLDER_NOT_FOUND, args, Some(4)),
                    }],
                    "{target:?}"
                );
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
        let claude = run(&lib, &agent("claude", T, &["P.yaml"]));
        assert_eq!(
            claude.problems,
            [BlockReason::Broken {
                item: format!("folder:{shown}"),
                via: via(&["P.yaml"]),
                reason: reason(FOLDER_PATH_CLI_UNSAFE, json!({ "path": shown }), Some(4)),
            }]
        );
        assert_eq!(claude.folders.len(), 1, "still a folder entry");
        assert!(run(&lib, &agent("codex", J, &["P.yaml"]))
            .problems
            .is_empty());
        assert!(run(&lib, &alone("P.yaml")).problems.is_empty());
    }

    // ── block ④ and the block ────────────────────────────────────────────────────────────────────────────

    // Block ④ stands alone: the whole resolution is that one row — no item, folder or notice computed from the catalog.
    #[test]
    fn a_library_preset_attached_without_a_usable_root_is_block_four() {
        let folder = absolute("f");
        let lib = library(&[
            ("P.yaml", &preset("")),
            (
                "Q.yaml",
                &preset(&format!(
                    "instructions:\n  - x.yaml\n  - gone.md\n  - ok.md\nskills:\n  - a\n  - b\nfolders:\n  - '{folder}'\n"
                )),
            ),
            ("x.yaml", "kind: preset\nversion: [\n"),
            ("ok.md", "ok"),
            ("a/SKILL.md", &skill("s", "1")),
            ("b/SKILL.md", &skill("s", "2")),
        ]);
        let raw = "../out.yaml";
        let unparsed = vec![(raw.to_owned(), AssetRef::parse(raw).unwrap_err())];
        // With a usable root, Q loads items, a folder and the default preset's notice, and shows ① ② ③ rows; the
        //   attached spelling that is not a reference is a ② row too.
        let mut usable = agent("claude", J, &["Q.yaml"]);
        usable.default_preset = true;
        usable.unparsed = unparsed.clone();
        let shown = run(&lib, &usable);
        assert_eq!(
            problems(&shown.problems),
            [
                row("x.yaml", "yaml-syntax"),
                row("gone.md", NOT_FOUND),
                row(raw, ITEM_PATH_INVALID),
                row("a+b", SKILL_NAME_CONFLICT),
            ],
            "what block ④ stands for"
        );
        assert_eq!(instructions(&shown), ["ok.md"]);
        assert_eq!(skill_names(&shown), ["s", "s"]);
        assert_eq!(shown.folders.len(), 1);
        assert_eq!(shown.notices.len(), 1, "the default preset's stand-in");
        for (root, message) in [
            (RootState::Unset, NO_ROOT),
            (RootState::Unavailable("missing".to_owned()), "missing"),
        ] {
            let alone = Resolution {
                problems: vec![BlockReason::LibraryUnavailable {
                    reason: reason(LIBRARY_UNAVAILABLE, json!({ "message": message }), None),
                }],
                ..Resolution::default()
            };
            // Every pass runs over one catalog holding these files: with `Unset` it stands for the empty catalog of an
            //   unset root (`absent.yaml` is not found), and with `Unavailable` it is the stale case.
            for attached in ["P.yaml", "absent.yaml", "Q.yaml"] {
                let mut stale = agent("claude", J, &[attached]);
                stale.default_preset = true;
                stale.unparsed = unparsed.clone();
                stale.root = root.clone();
                assert_eq!(run(&lib, &stale), alone, "{root:?}: {attached}");
            }
            let mut only_builtin = agent("claude", J, &[]);
            only_builtin.default_preset = true;
            only_builtin.root = root;
            assert!(
                run(&lib, &only_builtin).problems.is_empty(),
                "no library preset"
            );
        }
    }

    // Rows are left out only for block ④: with no library preset attached there is no block ④, so a library path the
    //   built-in default lists stays a not-found row.
    #[test]
    fn without_block_four_a_not_found_row_stays_even_without_a_usable_root() {
        let dir = crate::builtin::tests::folder(&[(
            crate::builtin::file_of(BuiltinId::DefaultPreset),
            "kind: preset\nversion: 1\ninstructions:\n  - x.md\n",
        )]);
        let lib = library(&[]);
        let builtins = Builtins::new(Some(dir.path()), &lib.backends);
        let mut target = agent("claude", J, &[]);
        target.default_preset = true;
        target.root = RootState::Unset;
        let resolution = resolve_with(&lib, &builtins, &target);
        assert_eq!(
            resolution.problems,
            [BlockReason::Missing {
                item: "x.md".to_owned(),
                via: via(&["engram:/default"]),
                reason: reason(NOT_FOUND, json!({ "path": "x.md" }), Some(4)),
            }]
        );
    }

    // TRD S25 §1-3 8: the block is the problem list itself — the same rows whoever launches, ordered ① ② ③ (block ④
    //   never shares the list — a_library_preset_attached_without_a_usable_root_is_block_four).
    #[test]
    fn the_block_is_the_problem_list_ordered_by_kind() {
        let (first, second) = (absolute("z-gone"), absolute("a-gone"));
        let lib = library(&[
            (
                "P.yaml",
                &preset(&format!(
                    "instructions:\n  - x.yaml\nskills:\n  - a\n  - b\nfolders:\n  - '{first}'\n  - '{second}'\n"
                )),
            ),
            ("x.yaml", "kind: preset\nversion: [\n"),
            ("a/SKILL.md", &skill("s", "1")),
            ("b/SKILL.md", &skill("s", "2")),
        ]);
        let mut resolution = run(&lib, &agent("claude", J, &["P.yaml"]));
        let (first, second) = (normalized(&first), normalized(&second));
        resolution.attach_folder_status(&HashMap::from([
            (path_key(&first), FolderStatus::Missing),
            (path_key(&second), FolderStatus::Missing),
        ]));
        let blocked = resolution.blocked().expect("blocked");
        assert_eq!(
            problems(&blocked.reasons),
            [
                row(
                    &format!("folder:{}", first.to_string_lossy()),
                    FOLDER_NOT_FOUND
                ),
                row(
                    &format!("folder:{}", second.to_string_lossy()),
                    FOLDER_NOT_FOUND
                ),
                row("x.yaml", "yaml-syntax"),
                row("a+b", SKILL_NAME_CONFLICT),
            ],
            "by kind, expansion order kept within a kind"
        );
        let mut rows = blocked.reasons.clone();
        rows.sort();
        let mut found = resolution.problems.clone();
        found.sort();
        assert_eq!(rows, found, "every problem row and nothing else");
        assert_eq!(run(&lib, &agent("claude", J, &[])).blocked(), None);
    }

    // ── review round 1 ───────────────────────────────────────────────────────────────────────────────────

    // U5 (a) with a repeated path: s1 is first reached tagged for claude, then again as common — a codex agent loads
    //   s1 and s2, so the preset on its own shows their conflict too.
    #[test]
    fn alone_a_skill_listed_again_keeps_every_backend_it_is_listed_for() {
        let lib = library(&[
            ("P.yaml", &preset("presets:\n  - A.yaml\n  - B.yaml\n")),
            (
                "A.yaml",
                &preset("skills:\n  - {path: s1, backend: claude}\n"),
            ),
            (
                "B.yaml",
                &preset("skills:\n  - s1\n  - {path: s2, backend: codex}\n"),
            ),
            ("s1/SKILL.md", &skill("dup", "1")),
            ("s2/SKILL.md", &skill("dup", "2")),
        ]);
        let codex = run(&lib, &agent("codex", J, &["P.yaml"]));
        assert_eq!(
            problems(&codex.problems),
            [row("s1+s2", SKILL_NAME_CONFLICT)]
        );
        let shown = run(&lib, &alone("P.yaml"));
        assert_eq!(
            problems(&shown.problems),
            [row("s1+s2", SKILL_NAME_CONFLICT)]
        );
        assert_eq!(
            shown.skills[0].2,
            via(&["P.yaml", "A.yaml"]),
            "first place kept"
        );
    }

    #[test]
    fn the_same_path_in_two_lists_is_judged_in_each() {
        let lib = library(&[
            (
                "P.yaml",
                &preset("instructions:\n  - a.md\nskills:\n  - a.md\n"),
            ),
            ("a.md", "a"),
        ]);
        let resolution = run(&lib, &agent("claude", J, &["P.yaml"]));
        assert_eq!(instructions(&resolution), ["a.md"]);
        assert_eq!(problems(&resolution.problems), [row("a.md", WRONG_KIND)]);
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
                    "instructions:\n  - a.md\nskills:\n  - bad-one\n  - {{path: s, backend: gamma}}\nfolders:\n  - '{blocked}'\n"
                )),
            ),
            ("a.md", "a"),
            ("bad-one/SKILL.md", &skill("bad-one", "d")),
            ("s/SKILL.md", &skill("s", "d")),
        ]);
        let terminal = run(&lib, &agent("gamma", T, &["P.yaml"]));
        assert_eq!(
            terminal.notices,
            [reason(
                "gamma-omitted",
                json!({ "instructions": ["a.md"], "skills": ["bad-one", "s"] }),
                None
            )]
        );
        assert!(terminal.instructions.is_empty() && terminal.skills.is_empty());
        let shown = normalized(&blocked).to_string_lossy().into_owned();
        let json_mode = run(&lib, &agent("gamma", J, &["P.yaml"]));
        assert_eq!(
            problems(&json_mode.problems),
            [
                row("bad-one", SKILL_NAME_INVALID),
                row(&format!("folder:{shown}"), FOLDER_PATH_CLI_UNSAFE),
            ]
        );
        assert_eq!(
            problems(&terminal.problems),
            [row(&format!("folder:{shown}"), FOLDER_PATH_CLI_UNSAFE)]
        );
        assert!(json_mode.notices.is_empty());
        assert!(run(&lib, &alone("P.yaml")).problems.is_empty());
    }
}
