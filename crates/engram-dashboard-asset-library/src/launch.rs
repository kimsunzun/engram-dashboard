//! Launch materials — the library service as the agent manager's `LaunchKitSource`: the launch check, the materials
//! a launch gets, and their removal; also the folder stat `agent.setWorkspace` asks outside any launch (one read
//! budget — ADR-0360 decision 4).
//!
//! ## Rules (invariants)
//!
//! - **One launch, one scope**: every check, the prepare and the working-folder call of one launch share one
//!   [`LaunchScope`] — one IO context (one required budget, one advisory allowance, one answer set) and the working
//!   folder the last check decided.
//! - **The check** (TRD S25 step-7 §3-8 `check_in`): each mount the profile's library items reach is stat'ed once per
//!   launch before anything in it is read (the attached presets' mounts in one batch, then each wave of mounts their
//!   references reach), and only a mount that stats `Ok` is read; the library files the profile reaches are read
//!   fresh with the store, each in its mount's folder (the overlay — local by assumption); two brief read-lock takes —
//!   the mounts and the gate's numbers before the overlay read, then `resolve` over the overlay, then the catalog
//!   (ADR-0324 decision 5). A mount that exists but cannot be read is block ④ naming it: the launch is refused with the
//!   rows found so far — no folder IO, no command-line check (ADR-0400 decision 19). No folder IO runs under the
//!   library lock. The required phase
//!   (folder-entry stats, listings and reads) runs first and alone; the advisory phase (the working-folder
//!   canonicalize, the backend's own skill roots, root canonicals) runs only when nothing blocks, on its own
//!   allowance; an advisory answer that is missing is never substituted (an unknown root is left out, an unknown
//!   working folder is the profile's own spelling).
//! - **Skips** (ADR-0387): what is absent is left out and the launch runs — a `missing` row (a library item not
//!   found, a folder entry not there), and every library item reached in a mount that is not registered or not there
//!   (its folder untouched), told as one notice per mount (ADR-0400 decision 19). Each skip is named in the plan's
//!   notices and, where the instruction channel allows it, in
//!   a launch note appended to the merged instructions (named, else counted, else none — measured with the text, so
//!   it never blocks). What exists but cannot be read or is broken still blocks.
//! - **The overlay is merged late, never removes**: the items read are handed to a short-lived merge job (the
//!   service's take-once hand-off), so the check never waits on the write lock.
//! - **The stamp** records the SHA-256 of every material source the plan uses; `prepare` re-reads exactly those with
//!   no library lock — gone or changed bytes are `Stale`, a folder read with no answer is `Failed`.
//! - **The volatile layout**: one launch's materials live in `<launch_root>\<id>-<epoch>\` — the merged instructions
//!   file [`INSTRUCTIONS_FILE`] written by `prepare`, and whatever the backend's materialize adds inside it.
//! - **Removal is best effort and bounded** ([`sweep_launch_dir`] · `release`): a depth-first walk that never follows
//!   a link — every link is detached (any depth), so a library skill folder a link points at survives — then files and
//!   folders are removed; what cannot be removed is logged and left (the boot sweep is the backstop). No wait, no
//!   retry, no lock, no panic.
// ADR-0324
// ADR-0334
// ADR-0336
// ADR-0341
// ADR-0353
// ADR-0355
// ADR-0360
// ADR-0387
// ADR-0400
// TRD S25 step-7 §3-8 · §4 · §6

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use engram_dashboard_agent::backend::settings_check::{
    CommandLine, FolderLoadKind, FolderLoadSet, NativePlaces, UnsureWhy,
};
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::{
    backend_word, escape_controls, BlockReason, LaunchBlocked, LaunchFs, LaunchKit,
    LaunchKitSource, LaunchPlan, LaunchScope, NoopLaunchKits, Notice, PrepareError, RawSetting,
    Reason, SkillRef, SourceRef, StdLaunchFs, FOLDER_ITEM_PREFIX, FOLDER_LISTING_PARTIAL,
    FOLDER_LISTING_SKIPPED, LIBRARY_UNAVAILABLE_SKIPPED, MISSING_ITEM_SKIPPED,
};
use engram_dashboard_agent::profile::{AgentOutputFormat, AgentProfile};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::{path_key, unlink_dir, PathKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::catalog::{key, Item, ItemContent, ScanItem};
use crate::folder::{slash_path, FolderListing, FolderStatus, LaunchIo, Need};
use crate::format::YamlFrontMatter;
use crate::gate::Tags;
use crate::gate::NOT_FOUND;
use crate::mounts::{admits_inside, Mount, MountName, Mounts};
use crate::path::{AssetRef, BuiltinId, LibPath};
use crate::resolve::{
    folder_item, resolve, CatalogLookup, ItemView, Looked, Lookup, MountStates, Resolution,
    RootState, TagRule, Target,
};

use crate::{
    extra_args, output_format, LibraryService, Want, LIBRARY_READ_CAP, LISTING_UNANSWERED,
};

/// The code of a library file a launch check reached but could not read (`args` = `{path, why}`) — problem ②: the
/// launch never runs on what an earlier read held.
// Added in step-7 P9 where the TRD was silent — TRD S25 step-7 B8 · B38.
pub const LIBRARY_READ_FAILED: &str = "library-read-failed";

/// The `PrepareError::Failed` prefix of a folder file `prepare` could not get an answer for — a slow read is not a
/// changed source (`Stale`).
pub const FOLDER_TIMEOUT: &str = "folder-timeout";

/// The merged instructions file in a launch's volatile folder.
// TRD S25 step-7 B14
pub const INSTRUCTIONS_FILE: &str = "instructions.md";

/// The one merge of a launch's instructions — the built-in priming first (when given), then each source in order.
/// Each part is its text without a leading BOM and without trailing line breaks; a source part starts with the line
/// `<!-- <source path> -->`; parts are joined by one blank line and the result ends with one line break. No part =
/// the empty string. The launch check measures this text and `prepare` writes it, so the two cannot differ.
pub fn merge_instructions(priming: Option<&str>, sources: &[(&str, &str)]) -> String {
    let body = |text: &str| {
        text.trim_start_matches('\u{feff}')
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    };
    let parts: Vec<String> = priming
        .map(body)
        .into_iter()
        .chain(
            sources
                .iter()
                .map(|(path, text)| format!("<!-- {path} -->\n{}", body(text))),
        )
        .collect();
    let mut merged = parts.join("\n\n");
    if !merged.is_empty() {
        merged.push('\n');
    }
    merged
}

/// `merged` (the text of [`merge_instructions`]) with the launch note `note` as its last part — after one blank line,
/// ending with one line break.
pub(crate) fn with_note(merged: &str, note: &str) -> String {
    if merged.is_empty() {
        format!("{note}\n")
    } else {
        format!("{merged}\n{note}\n")
    }
}

/// What a launch that runs leaves out: the library items of each absent mount (with why), and every other skipped row
/// — a `not-found` library item or a `folder-not-found` folder entry.
// ADR-0374
// ADR-0387
// ADR-0400 decision 19
struct Skips {
    /// Per absent mount, in first-reach order: (mount, message, its items in expansion order).
    mounts: Vec<(String, String, Vec<String>)>,
    rows: Vec<SkipRow>,
}

struct SkipRow {
    item: String,
    via: Vec<String>,
    line: Option<u32>,
}

impl Skips {
    /// The skip set of `resolution`: the `not-found` rows of a mount it found absent fold into that mount's one notice.
    fn of(resolution: &Resolution) -> Skips {
        let mut mounts: Vec<(String, String, Vec<String>)> = resolution
            .absent_mounts
            .iter()
            .map(|(mount, message)| (mount.clone(), message.clone(), Vec::new()))
            .collect();
        let mut rows = Vec::new();
        for problem in resolution.skipped() {
            let BlockReason::Missing { item, via, reason } = problem else {
                continue;
            };
            let first = item.split('/').next().unwrap_or_default();
            let absent = mounts
                .iter_mut()
                .find(|(mount, _, _)| path_key(Path::new(mount)) == path_key(Path::new(first)));
            match absent {
                Some((_, _, items)) if reason.code == NOT_FOUND => items.push(item),
                _ => rows.push(SkipRow {
                    item,
                    via,
                    line: reason.line,
                }),
            }
        }
        Skips { mounts, rows }
    }

    fn count(&self) -> usize {
        self.rows.len()
            + self
                .mounts
                .iter()
                .map(|(_, _, items)| items.len())
                .sum::<usize>()
    }

    /// The reply notices: one per absent mount first, then one per row.
    fn notices(&self) -> Vec<Notice> {
        let root = self.mounts.iter().map(|(mount, message, items)| {
            notice(
                LIBRARY_UNAVAILABLE_SKIPPED,
                json!({ "mount": mount, "message": message, "items": items }),
            )
        });
        let rows = self.rows.iter().map(|row| Reason {
            code: MISSING_ITEM_SKIPPED.to_owned(),
            args: json!({ "item": row.item, "via": row.via }),
            line: row.line,
        });
        root.chain(rows).collect()
    }

    /// The launch note that names every item; `None` = nothing was skipped.
    fn named_note(&self) -> Option<String> {
        if self.count() == 0 {
            return None;
        }
        let mut lines = Vec::new();
        if !self.rows.is_empty() {
            lines.push(
                "Engram launch note: these items your presets reference were not found and were left out of this \
                 launch:"
                    .to_owned(),
            );
            for row in &self.rows {
                let at = match row.via.last() {
                    Some(preset) => format!("listed in {}", escape_controls(preset)),
                    None => "attached to this agent".to_owned(),
                };
                lines.push(match row.item.strip_prefix(FOLDER_ITEM_PREFIX) {
                    Some(path) => format!(
                        "- folder {} (not found or not a folder; {at})",
                        escape_controls(path)
                    ),
                    None => format!("- {} ({at})", escape_controls(&row.item)),
                });
            }
        }
        for (mount, message, items) in &self.mounts {
            let items: Vec<String> = items.iter().map(|item| escape_controls(item)).collect();
            let sentence = format!(
                "The library mount {} could not be used ({}), so these library items were left out: {}",
                escape_controls(mount),
                escape_controls(message),
                items.join(", ")
            );
            lines.push(match lines.is_empty() {
                true => format!("Engram launch note: {sentence}"),
                false => sentence,
            });
        }
        Some(lines.join("\n"))
    }

    /// The launch note that only counts ([`count_note`]); `None` = nothing was skipped.
    fn count_note(&self) -> Option<String> {
        (self.count() > 0).then(|| count_note(self.count()))
    }
}

/// The launch note that only counts `n` skipped items — one fixed ASCII sentence, no name.
pub(crate) fn count_note(n: usize) -> String {
    match n {
        1 => "Engram launch note: 1 item your presets reference was not found and was left out of this launch."
            .to_owned(),
        n => format!(
            "Engram launch note: {n} items your presets reference were not found and were left out of this launch."
        ),
    }
}

/// The lowercase hex SHA-256 of `bytes` — the stamp of a source, the instructions hash and a preset's `rev`.
// ADR-0336
// TRD S25 step-7 B7 · B9
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The volatile folder of launch `(id, epoch)`.
pub(crate) fn volatile_dir(launch_root: &Path, id: AgentId, epoch: u32) -> PathBuf {
    launch_root.join(format!("{id}-{epoch}"))
}

/// One launch's state, held in its [`LaunchScope`].
struct Launch {
    io: Arc<LaunchIo>,
    /// The working folder the last check of this launch decided: (`profile.cwd` as it was given, the folder decided).
    cwd: Mutex<Option<(PathBuf, PathBuf)>>,
}

/// The library files one check read fresh (step 3), by catalog key of the path asked.
#[derive(Default)]
struct Overlay {
    /// The mounts the files were read in.
    mounts: Mounts,
    seen: HashMap<PathKey, Seen>,
    /// Every item the reads made — the late merge's positive observations.
    items: Vec<Item>,
    /// The bytes behind each item read, by the path key of the item's disk path (a skill: its folder).
    files: HashMap<PathKey, FileRead>,
}

/// The stat of each mount a launch reached, by name key — taken once per launch, before anything in it is read.
#[derive(Default)]
struct MountStats(HashMap<PathKey, (MountName, FolderStatus)>);

impl MountStats {
    /// `Some(true)` = `mount` stat'ed `Ok`; `Some(false)` = stat'ed, not `Ok`; `None` = not stat'ed yet.
    fn status(&self, mount: &Mount) -> Option<bool> {
        self.0
            .get(&mount.name.key())
            .map(|(_, status)| *status == FolderStatus::Ok)
    }

    /// The mount states a launch resolves under: each mount stat'ed judged by its stat alone ([`RootState::of`] — a
    /// launch reads the files fresh, so no first-scan answer decides), an absent one's items as ① rows
    /// ([`MountStates::absent_items`]).
    // ADR-0387
    // ADR-0400 decision 19
    fn states(&self, mounts: &Mounts) -> MountStates {
        let mut states = MountStates::new(mounts, true);
        for (name, status) in self.0.values() {
            states.set(name, RootState::of(&Ok(()), Some(status)));
        }
        states
    }
}

/// What one library path asked by the overlay turned out to be.
enum Seen {
    /// `Overlay::items[i]`.
    Item(usize),
    /// No library item at that path — gone, not admitted by the scan's name rule, at or below a link inside the
    /// library, or not a file kind the library holds.
    Absent,
    /// Its read failed, with why.
    Failed(String),
}

/// One file a check read.
struct FileRead {
    file: PathBuf,
    sha256: String,
    /// The text (not UTF-8 → U+FFFD).
    text: String,
}

/// The overlay first, then the catalog (and the built-ins through it) for anything the overlay did not ask.
struct OverlayLookup<'a> {
    overlay: &'a Overlay,
    catalog: CatalogLookup<'a>,
}

impl Lookup for OverlayLookup<'_> {
    fn item(&self, r: &AssetRef) -> Looked<'_> {
        let AssetRef::Library(path) = r else {
            return self.catalog.item(r);
        };
        match self.overlay.seen.get(&key(path)) {
            Some(Seen::Item(i)) => {
                let item = &self.overlay.items[*i];
                Looked::Present(ItemView {
                    kind: &item.kind,
                    content: &item.content,
                    file: self.overlay.mounts.disk_path(&item.path),
                    stand_in: None,
                })
            }
            Some(Seen::Absent) => Looked::Missing,
            Some(Seen::Failed(why)) => Looked::Unreadable(Reason {
                code: LIBRARY_READ_FAILED.to_owned(),
                args: json!({ "path": path.as_str(), "why": why }),
                line: None,
            }),
            None => self.catalog.item(r),
        }
    }
}

/// A folder entry the required phase listed and the port read.
struct Listed {
    folder: PathBuf,
    /// [`crate::resolve::ResolvedFolder::instructions_at`].
    instructions_at: usize,
    set: FolderLoadSet,
    /// The texts the port's reads got, by folder-relative path.
    texts: HashMap<PathBuf, String>,
}

/// The reader handed to the port for one folder (folder-relative paths, joined to the folder and keyed by the
/// absolute path on the launch's budget). A read that got no answer about the file is remembered — whatever the port
/// files it as, the caller decides what it means for the folder.
pub(crate) struct FolderReads<'a> {
    svc: &'a LibraryService,
    io: &'a LaunchIo,
    root: &'a Path,
    need: Need,
    pub(crate) unanswered: RefCell<Option<String>>,
    texts: RefCell<HashMap<PathBuf, String>>,
}

impl<'a> FolderReads<'a> {
    pub(crate) fn new(
        svc: &'a LibraryService,
        io: &'a LaunchIo,
        root: &'a Path,
        need: Need,
    ) -> Self {
        FolderReads {
            svc,
            io,
            root,
            need,
            unanswered: RefCell::new(None),
            texts: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn read(&self, rel: &Path) -> io::Result<String> {
        let read = self
            .svc
            .io
            .read_one(self.io, &self.root.join(rel), self.need, false);
        match &read {
            Ok(text) => {
                self.texts
                    .borrow_mut()
                    .insert(rel.to_path_buf(), text.clone());
            }
            // `read_one`: no answer about the file (timeout, not checked, a failed worker) is `TimedOut`.
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                self.unanswered
                    .borrow_mut()
                    .get_or_insert_with(|| e.to_string());
            }
            Err(_) => {}
        }
        read
    }
}

/// What `prepare` re-reads and compares (`LaunchPlan::stamp`, opaque to everyone else), and the launch note it writes.
#[derive(Serialize, Deserialize)]
struct Stamp {
    files: Vec<StampEntry>,
    /// The launch note the check measured as the last part of the merged instructions; `None` = no note.
    // ADR-0387
    #[serde(default)]
    note: Option<String>,
    /// The command-line refusal code that dropped a note the launch would have carried (both forms refused, the
    /// text without one fits) — `prepare` logs it once.
    #[serde(default)]
    dropped_note: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StampEntry {
    file: String,
    sha256: String,
    /// The folder entry the file is in — read through the bounded folder IO; `None` = a library file, read with the
    /// store.
    folder: Option<String>,
}

impl StampEntry {
    fn library(read: &FileRead) -> Self {
        StampEntry {
            file: read.file.to_string_lossy().into_owned(),
            sha256: read.sha256.clone(),
            folder: None,
        }
    }
}

impl LaunchKitSource for LibraryService {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        self.check_in(&self.begin_launch(), profile)
    }

    fn prepare(
        &self,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        self.prepare_in(&self.begin_launch(), plan, id, epoch)
    }

    fn release(&self, id: AgentId, epoch: u32) {
        release_dir(&self.values.launch_root, id, epoch);
    }

    fn begin_launch(&self) -> LaunchScope {
        LaunchScope::new(Arc::new(self.new_launch()))
    }

    fn check_in(
        &self,
        scope: &LaunchScope,
        profile: &AgentProfile,
    ) -> Result<LaunchPlan, LaunchBlocked> {
        self.launch_check(&self.launch_of(scope), profile)
    }

    fn prepare_in(
        &self,
        scope: &LaunchScope,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        let Some(stamp) = plan.stamp.as_deref() else {
            return NoopLaunchKits.prepare(plan, id, epoch);
        };
        let launch = self.launch_of(scope);
        let dir = volatile_dir(&self.values.launch_root, id, epoch);
        fs::create_dir_all(&dir).map_err(|e| {
            PrepareError::Failed(format!("could not create the volatile folder {dir:?}: {e}"))
        })?;
        let stamp: Stamp = serde_json::from_str(stamp)
            .map_err(|e| PrepareError::Failed(format!("the plan's stamp does not read: {e}")))?;
        // ADR-0387: the reply notice and the log still name the skip; only the agent is not told.
        if let Some(code) = &stamp.dropped_note {
            tracing::warn!(
                agent = %id,
                epoch,
                code = %code,
                "launch note dropped; command line cannot carry it"
            );
        }
        let mut texts = HashMap::new();
        for entry in &stamp.files {
            let (sha256, text) = self.reread(&launch.io, entry)?;
            if sha256 != entry.sha256 {
                return Err(PrepareError::Stale(format!(
                    "changed since the check: {}",
                    entry.file
                )));
            }
            texts.insert(path_key(Path::new(&entry.file)), text);
        }
        let sources = plan
            .instructions
            .iter()
            .map(|source| {
                texts
                    .get(&path_key(&source.file))
                    .map(|text| (source.path.as_str(), text.as_str()))
                    .ok_or_else(|| {
                        PrepareError::Failed(format!(
                            "the check did not read the instruction source {:?}",
                            source.file
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let priming = plan.include_priming.then(|| self.builtins.priming_text());
        let parts = priming.is_some() || !sources.is_empty() || stamp.note.is_some();
        let (instructions_file, instructions_hash) = if parts {
            let merged = merge_instructions(priming, &sources);
            let merged = match &stamp.note {
                Some(note) => with_note(&merged, note),
                None => merged,
            };
            let file = dir.join(INSTRUCTIONS_FILE);
            fs::write(&file, &merged)
                .map_err(|e| PrepareError::Failed(format!("could not write {file:?}: {e}")))?;
            (Some(file), Some(sha256_hex(merged.as_bytes())))
        } else {
            (None, None)
        };
        Ok(LaunchKit {
            skills: plan.skills.clone(),
            plan,
            volatile_dir: Some(dir),
            instructions_file,
            instructions_hash,
            last_applied_instructions_hash: None,
        })
    }

    fn working_folder(&self, scope: &LaunchScope, cwd: &Path) -> PathBuf {
        let launch = self.launch_of(scope);
        let decided = lock(&launch.cwd).clone();
        match decided {
            Some((asked, decided)) if asked == cwd => decided,
            _ => self
                .canonical_cwd(&launch.io, cwd)
                .unwrap_or_else(|| cwd.to_path_buf()),
        }
    }

    // ADR-0360: a folder read outside a launch gets the 5 s read budget (decision 4) — a dead share answers `timeout`.
    // ADR-0386
    fn stat_folder(&self, folder: &Path) -> Result<(), String> {
        self.stat_words(&self.io.begin_read(), folder)
    }
}

impl LibraryService {
    fn new_launch(&self) -> Launch {
        Launch {
            io: self.io.begin_launch(),
            cwd: Mutex::new(None),
        }
    }

    /// The launch a scope holds. A scope this service did not open is a caller bug: logged (and asserted in a debug
    /// build); a release build goes on with a fresh launch.
    fn launch_of(&self, scope: &LaunchScope) -> Arc<Launch> {
        scope.get::<Launch>().unwrap_or_else(|| {
            tracing::warn!("unknown LaunchScope; using a new LaunchIo");
            debug_assert!(false, "a launch scope this library did not open");
            Arc::new(self.new_launch())
        })
    }

    /// The launch check of `profile` on `launch` (module rules; TRD S25 step-7 §3-8 steps 1–9).
    fn launch_check(
        &self,
        launch: &Launch,
        profile: &AgentProfile,
    ) -> Result<LaunchPlan, LaunchBlocked> {
        let backends = &*self.values.backends;
        let word = backend_word(&profile.command);
        // B13: a backend with no settings check takes no preset.
        let (Some(check), Some(want)) = (backends.for_word(&word), self.want_of(profile)) else {
            return NoopLaunchKits.check(profile);
        };
        let mode = output_format(&profile.command);
        let codes = check.notice_codes(mode);
        let io = &*launch.io;

        // ADR-0324 decision 5: the numbers are noted in the read lock that reads the mounts the overlay reads in.
        let (noted, mounts) = self
            .gate
            .with_read(|state| (self.gate.numbers(), state.mounts.clone()));
        let mut stats = MountStats::default();
        let overlay = self.read_overlay(io, &mounts, &want, &mut stats);
        let target = Target {
            backend: want.backend,
            mode: want.mode,
            default_preset: want.default_preset,
            presets: want.presets,
            unparsed: want.unparsed,
            mounts: stats.states(&mounts),
        };
        let mut resolution = self.gate.with_read(|state| {
            let lookup = OverlayLookup {
                overlay: &overlay,
                catalog: CatalogLookup {
                    catalog: &state.catalog,
                    mounts: &state.mounts,
                    builtins: &self.builtins,
                },
            };
            resolve(&target, &lookup, backends)
        });
        // ADR-0361
        // ADR-0387 · ADR-0400 decision 19: a launch's ④ rows are mounts that exist but cannot be read (an absent
        //   mount's items are skipped rows). User decision 2026-11-12: a launch reaching one runs no folder IO and no
        //   command-line judgment.
        if resolution
            .problems
            .iter()
            .any(|problem| matches!(problem, BlockReason::LibraryUnavailable { .. }))
        {
            if let Some(blocked) = resolution.blocked() {
                return Err(blocked);
            }
        }
        self.mark_links(&mounts, &mut resolution);
        // ADR-0366
        // B26: the merge needs nothing the later steps find, so it is handed off as soon as the overlay is complete.
        if !overlay.items.is_empty() {
            self.hand_off_merge(noted, overlay.items.clone());
        }

        let (listed, mut notices) = self.folder_phase(io, check, mode, &mut resolution);

        // B30: a launch already blocked runs no advisory IO; its working folder is the profile's own spelling. Skipped
        //   rows do not block (ADR-0387).
        let canonical = if resolution.blocked().is_none() {
            self.canonical_cwd(io, &profile.cwd)
        } else {
            None
        };
        if let (Some(code), Some(cwd)) = (codes.same_name, &canonical) {
            let names =
                self.same_names(io, check, mode, profile, cwd, &mounts, &resolution, &listed);
            notices.extend(
                names
                    .into_iter()
                    .map(|(name, paths)| notice(code, json!({ "name": name, "paths": paths }))),
            );
        }
        let cwd = canonical.unwrap_or_else(|| profile.cwd.clone());

        let mut sources: Vec<(SourceRef, String)> = resolution
            .instructions
            .iter()
            .map(|(path, file)| {
                let text = overlay.read_of(file).map(|read| read.text.clone());
                let source = SourceRef {
                    path: path.clone(),
                    file: file.clone(),
                };
                (source, text.unwrap_or_default())
            })
            .collect();
        let mut stamp: Vec<StampEntry> = overlay
            .seen
            .values()
            .filter_map(|seen| match seen {
                Seen::Item(i) => Some(&overlay.items[*i]),
                Seen::Absent | Seen::Failed(_) => None,
            })
            .filter(|item| {
                matches!(
                    item.content,
                    ItemContent::Preset { .. } | ItemContent::Settings { .. }
                )
            })
            .filter_map(|item| {
                let file = overlay.mounts.disk_path(&item.path)?;
                overlay.read_of(&file)
            })
            .chain(
                resolution
                    .instructions
                    .iter()
                    .filter_map(|(_, file)| overlay.read_of(file)),
            )
            .chain(
                resolution
                    .skills
                    .iter()
                    .filter_map(|(_, dir, _)| overlay.read_of(dir)),
            )
            .map(StampEntry::library)
            .collect();
        let mut extra_skill_roots = Vec::new();
        // Parent TRD S25 §1-5: a folder's instruction file goes right after its preset's own instruction lines. The
        //   folders come in expansion order, so each insert lands after those of earlier folders at the same place.
        let mut inserted = 0;
        for Listed {
            folder,
            instructions_at,
            set,
            texts,
        } in &listed
        {
            for row in &set.carried {
                let file = folder.join(&row.path);
                match row.kind {
                    FolderLoadKind::Instructions => {
                        // The port carries an instruction file only after reading it.
                        let text = texts.get(&row.path).cloned().unwrap_or_default();
                        stamp.push(StampEntry {
                            file: file.to_string_lossy().into_owned(),
                            sha256: sha256_hex(text.as_bytes()),
                            folder: Some(folder.to_string_lossy().into_owned()),
                        });
                        let source = SourceRef {
                            path: folder_item(folder),
                            file,
                        };
                        sources.insert(instructions_at + inserted, (source, text));
                        inserted += 1;
                    }
                    FolderLoadKind::SkillRoot => extra_skill_roots.push(file),
                    FolderLoadKind::Rule
                    | FolderLoadKind::Skill
                    | FolderLoadKind::Agent
                    | FolderLoadKind::Command => {}
                }
            }
            if let (Some(code), false) = (codes.folder_omitted, set.omitted.is_empty()) {
                let items: Vec<String> = set
                    .omitted
                    .iter()
                    .map(|row| slash_path(&row.path))
                    .collect();
                let args = json!({ "folder": folder.to_string_lossy(), "items": items });
                notices.push(notice(code, args));
            }
        }

        let config_args: Vec<String> = resolution
            .raw_settings
            .iter()
            // A refusal is the settings asset's ② row already: its verdict ran this call on the same value.
            .filter_map(|settings| check.command_line_args(&settings.value).ok())
            .flatten()
            .collect();
        let priming = resolution
            .include_priming
            .then(|| self.builtins.priming_text());
        let pairs: Vec<(&str, &str)> = sources
            .iter()
            .map(|(source, text)| (source.path.as_str(), text.as_str()))
            .collect();
        let merged = merge_instructions(priming, &pairs);
        let extra = extra_args(&profile.command);
        let fits = |text: &str| {
            check.command_line_fits(&CommandLine {
                mode,
                cwd: &cwd,
                extra_args: &extra,
                config_args: &config_args,
                instructions: (!text.is_empty()).then_some(text),
            })
        };
        let skips = Skips::of(&resolution);
        // ADR-0387: the note is the merged text's last part, measured with it, so it never turns a skip into a block:
        //   the named form, else the count form, else none. It never opens a replacing channel by itself — with no
        //   instruction text of Engram's own (blank as codex reads it), a backend whose instructions replace the
        //   user's own (ADR-0215) gets none.
        let forms = if merged.trim().is_empty() && !check.instructions_append(mode) {
            Vec::new()
        } else {
            [skips.named_note(), skips.count_note()]
                .into_iter()
                .flatten()
                .collect()
        };
        let (mut note, mut refused) = (None, None);
        for form in forms {
            match fits(&with_note(&merged, &form)) {
                Ok(()) => {
                    note = Some(form);
                    break;
                }
                Err(reason) => {
                    refused.get_or_insert(reason.code);
                }
            }
        }
        let mut dropped_note = None;
        if note.is_none() {
            // ADR-0361
            match fits(&merged) {
                Err(reason) => resolution.problems.push(BlockReason::Broken {
                    item: format!("command-line:{word}"),
                    via: Vec::new(),
                    reason,
                }),
                Ok(()) => dropped_note = refused,
            }
        }
        // ADR-0353: the backend's translation applies the same rule to the folder the spawn gets.
        // ADR-0361
        if let Some(reason) = check.working_folder_blocks(&cwd, mode) {
            resolution.problems.push(BlockReason::Broken {
                item: format!("working-folder:{}", cwd.to_string_lossy()),
                via: Vec::new(),
                reason,
            });
        }
        *lock(&launch.cwd) = Some((profile.cwd.clone(), cwd));

        if let Some(blocked) = resolution.blocked() {
            return Err(blocked);
        }
        // ADR-0387: skip notices first, then the resolution's, then the folder and same-name notices.
        notices.splice(0..0, skips.notices().into_iter().chain(resolution.notices));
        // ADR-0387: a folder entry that is not there is left out of the plan; a folder with no status stays.
        let skipped_folders: Vec<String> = skips
            .rows
            .iter()
            .filter(|row| row.item.starts_with(FOLDER_ITEM_PREFIX))
            .map(|row| row.item.clone())
            .collect();
        let stamp = serde_json::to_string(&Stamp {
            files: stamp,
            note,
            dropped_note,
        })
        .expect("a stamp of strings serializes");
        Ok(LaunchPlan {
            backend: word,
            instructions: sources.into_iter().map(|(source, _)| source).collect(),
            skills: resolution
                .skills
                .into_iter()
                .map(|(name, dir, _)| SkillRef { name, dir })
                .collect(),
            raw_settings: resolution
                .raw_settings
                .into_iter()
                .map(|settings| RawSetting {
                    preset: settings
                        .via
                        .last()
                        .cloned()
                        .unwrap_or_else(|| settings.asset.spelling()),
                    value: settings.value,
                })
                .collect(),
            include_priming: resolution.include_priming,
            notices,
            real: true,
            folders: resolution
                .folders
                .into_iter()
                .filter(|folder| !skipped_folders.contains(&folder_item(&folder.path)))
                .map(|folder| folder.path)
                .collect(),
            extra_skill_roots,
            stamp: Some(stamp),
        })
    }

    /// Step 3's library half: every library preset `want` reaches and every library file those presets list, read
    /// fresh with the store through the partial scan a refresh of that path runs — so a path is admitted exactly as
    /// the scan admits it (names, links, kinds) and each read draws its number first. References are followed by the
    /// resolution's tag rule ([`TagRule::refs`]): an entry only another backend's tag lists is neither read nor its
    /// mount stat'ed. A path is read only once its mount stats `Ok` in `stats`: the mounts not stat'ed yet are stat'ed
    /// in one batch per wave (first the attached presets', then those their references reach), and nothing in a mount
    /// that does not stat `Ok`, or in no registered mount, is read — resolution judges its items by the mount
    /// ([`MountStats::states`]).
    // ADR-0365
    // ADR-0400 decision 19
    fn read_overlay(
        &self,
        io: &LaunchIo,
        mounts: &Mounts,
        want: &Want,
        stats: &mut MountStats,
    ) -> Overlay {
        let mut overlay = Overlay {
            mounts: mounts.clone(),
            ..Overlay::default()
        };
        let rule = TagRule::of(&*self.values.backends, want.backend.as_deref(), want.mode);
        let mut todo: Vec<LibPath> = want
            .presets
            .iter()
            .filter_map(|preset| match preset {
                AssetRef::Library(path) => Some(path.clone()),
                AssetRef::BuiltIn(_) => None,
            })
            .collect();
        if want.default_preset {
            if let ItemContent::Preset { doc, .. } =
                self.builtins.view(BuiltinId::DefaultPreset).content
            {
                todo.extend(rule.refs(doc).into_iter().cloned());
            }
        }
        loop {
            let mut waiting: Vec<LibPath> = Vec::new();
            while let Some(path) = todo.pop() {
                if overlay.seen.contains_key(&key(&path)) {
                    continue;
                }
                match mounts.locate(&path).map(|(mount, _)| stats.status(mount)) {
                    // Resolution looks nothing up there: a mount not registered, or one that cannot be used.
                    None | Some(Some(false)) => continue,
                    Some(None) => {
                        waiting.push(path);
                        continue;
                    }
                    Some(Some(true)) => {}
                }
                let seen = self.overlay_read(&path, &mut overlay);
                if let Seen::Item(i) = &seen {
                    if let ItemContent::Preset { doc, .. } = &overlay.items[*i].content {
                        todo.extend(rule.refs(doc).into_iter().cloned());
                    }
                }
                overlay.seen.insert(key(&path), seen);
            }
            if waiting.is_empty() {
                return overlay;
            }
            let wave: Vec<Mount> = waiting
                .iter()
                .filter_map(|path| mounts.locate(path).map(|(mount, _)| mount.clone()))
                .collect();
            self.stat_wave(io, &wave, stats);
            todo = waiting;
        }
    }

    /// Stats the folders of `wave` (each mount once) as required asks on `io`, in one batch, into `stats`.
    // ADR-0360
    fn stat_wave(&self, io: &LaunchIo, wave: &[Mount], stats: &mut MountStats) {
        let mut seen = std::collections::HashSet::new();
        let wave: Vec<&Mount> = wave
            .iter()
            .filter(|mount| seen.insert(mount.name.key()))
            .collect();
        let wave: Vec<Mount> = wave.into_iter().cloned().collect();
        let statuses = self.stat_mount_list(io, &wave);
        for (mount, status) in wave.into_iter().zip(statuses) {
            stats
                .0
                .insert(mount.name.key(), (mount.name.clone(), status));
        }
    }

    /// One library path read for the overlay in its mount's folder; the items it makes and their bytes go into
    /// `overlay`. A path in no registered mount, or a mount's top, is absent (a mount's top is no item).
    fn overlay_read(&self, path: &LibPath, overlay: &mut Overlay) -> Seen {
        // The scan never admits such a name, so the catalog never holds it either.
        if !admits_inside(path) {
            return Seen::Absent;
        }
        let Some((mount, Some(under))) = overlay.mounts.locate(path) else {
            return Seen::Absent;
        };
        let mount = mount.clone();
        let bytes = RefCell::new(HashMap::new());
        let read = |file: &Path| {
            let read = self.store.read(file, LIBRARY_READ_CAP)?;
            bytes.borrow_mut().insert(path_key(file), read.clone());
            Ok(read)
        };
        let result = match self.build_scan_reading(&mount, Some(&under), &Tags::new(), &read) {
            Ok(result) => result,
            Err(e) => return Seen::Failed(e.to_string()),
        };
        let bytes = bytes.into_inner();
        let mut found = None;
        for entry in result.entries {
            let ScanItem::Fresh(item) = entry else {
                continue;
            };
            let Some(disk) = overlay.mounts.disk_path(&item.path) else {
                continue;
            };
            let file = match &item.content {
                ItemContent::Skill { file_name, .. } => disk.join(file_name),
                _ => disk.clone(),
            };
            if let Some(read) = bytes.get(&path_key(&file)) {
                let text = String::from_utf8_lossy(read).into_owned();
                let read = FileRead {
                    file,
                    sha256: sha256_hex(read),
                    text,
                };
                overlay.files.insert(path_key(&disk), read);
            }
            if key(&item.path) == key(path) {
                found = Some(overlay.items.len());
            }
            overlay.items.push(item);
        }
        match (found, result.errors.first()) {
            (Some(i), _) => Seen::Item(i),
            (None, None) => Seen::Absent,
            (None, Some((_, why))) => Seen::Failed(why.clone()),
        }
    }

    /// Step 5 — the required phase: the folder entries' stats, then (when the backend needs them at launch) their
    /// listings and the port's reads of their files. A folder that is missing or not a folder is a ① row (skipped); one
    /// that cannot be read, whose listing fails for another reason than a missing folder, or one of whose reads gets no
    /// answer is a ② `folder-unreadable` row; a listing that finds the folder gone is a ① row (skipped).
    /// A file below a listed folder that the port could not read for another reason is named in that folder's
    /// `folder-listing-skipped` notice. Answers the folders listed and their listing notices.
    // ADR-0387
    fn folder_phase(
        &self,
        io: &LaunchIo,
        check: &dyn SettingsCheck,
        mode: AgentOutputFormat,
        resolution: &mut Resolution,
    ) -> (Vec<Listed>, Vec<Notice>) {
        let required = |paths: Vec<PathBuf>| -> Vec<(PathBuf, Need)> {
            paths
                .into_iter()
                .map(|path| (path, Need::Required))
                .collect()
        };
        let statuses = self.io.stat_all(io, &required(resolution.folder_paths()));
        resolution.attach_folder_status(&statuses);
        if !check.folder_listing_at_launch(mode) {
            return (Vec::new(), Vec::new());
        }
        let healthy: Vec<(PathBuf, usize)> = resolution
            .folders
            .iter()
            .filter(|folder| folder.status == Some(FolderStatus::Ok))
            .map(|folder| (folder.path.clone(), folder.instructions_at))
            .collect();
        let paths = healthy.iter().map(|(folder, _)| folder.clone()).collect();
        let mut listings = self.io.list_all(io, &required(paths));
        let (mut listed, mut notices, mut failed) = (Vec::new(), Vec::new(), HashMap::new());
        for (folder, instructions_at) in healthy {
            let folder_key = path_key(&folder);
            // ADR-0293: `list_all` answers every path it is asked, but a broken answer is an unreadable folder here
            //   (a ② `folder-unreadable` row), never an abort.
            let listing = listings.remove(&folder_key).unwrap_or_else(|| {
                tracing::error!(
                    path = %escape_controls(&folder.to_string_lossy()),
                    "the bounded listing gave no answer for the path it was asked"
                );
                Err(FolderStatus::Unreadable(LISTING_UNANSWERED.to_owned()))
            });
            let listing = match listing {
                Ok(listing) => listing,
                Err(status) => {
                    failed.insert(folder_key, status);
                    continue;
                }
            };
            let reads = FolderReads::new(self, io, &folder, Need::Required);
            let set = check.folder_load_set(
                &listing.entries,
                &|rel| reads.read(rel),
                &YamlFrontMatter,
                mode,
            );
            let unanswered = reads.unanswered.into_inner();
            // ADR-0387: a file the port could not read (an answer, not a timeout) is skipped and named.
            let unreadable: Vec<String> = match unanswered {
                Some(_) => Vec::new(),
                None => set
                    .unsure
                    .iter()
                    .filter(|unsure| matches!(unsure.why, UnsureWhy::Unreadable(_)))
                    .map(|unsure| slash_path(&unsure.path))
                    .collect(),
            };
            notices.extend(listing_notices(&folder, &listing, &unreadable));
            match unanswered {
                Some(why) => {
                    failed.insert(folder_key, FolderStatus::Unreadable(why));
                }
                None => listed.push(Listed {
                    texts: reads.texts.into_inner(),
                    folder,
                    instructions_at,
                    set,
                }),
            }
        }
        resolution.attach_folder_status(&failed);
        (listed, notices)
    }

    /// The working folder's canonical path on the advisory allowance; `None` (logged only — B29) when it did not
    /// answer or answered an error.
    fn canonical_cwd(&self, io: &LaunchIo, cwd: &Path) -> Option<PathBuf> {
        let answer = self
            .io
            .canonical_all(io, &[(cwd.to_path_buf(), Need::Advisory)])
            .remove(&path_key(cwd));
        match answer {
            Some(Ok(canonical)) => Some(canonical),
            other => {
                tracing::warn!(
                    cwd = %escape_controls(&cwd.to_string_lossy()),
                    "cwd canonicalize unanswered or failed; using profile path: {other:?}"
                );
                None
            }
        }
    }

    /// Step 6 (b) and the same-name comparison of step 7: each skill name found in more than one place, with the
    /// places (canonical skill folders, sorted). The places = the library skills the plan carries, the skills of each
    /// listed folder entry and of each of the backend's own skill roots found from the canonical `cwd` — each root
    /// (each mount's folder too) by its canonical path, asked on the advisory allowance, and each place counted once by
    /// its path key; a root whose canonical path is unknown is left out (logged), never compared by its spelling
    /// (ADR-0355).
    #[allow(clippy::too_many_arguments)]
    fn same_names(
        &self,
        io: &LaunchIo,
        check: &dyn SettingsCheck,
        mode: AgentOutputFormat,
        profile: &AgentProfile,
        cwd: &Path,
        mounts: &Mounts,
        resolution: &Resolution,
        listed: &[Listed],
    ) -> Vec<(String, Vec<String>)> {
        let mut roots: Vec<(PathBuf, Vec<(String, PathBuf)>)> = listed
            .iter()
            .map(|listed| {
                let names = listed
                    .set
                    .skill_names
                    .iter()
                    .map(|skill| (skill.name.clone(), skill_folder(&skill.path)))
                    .collect();
                (listed.folder.clone(), names)
            })
            .collect();
        roots.extend(self.native_roots(io, check, mode, profile, cwd));
        for mount in mounts.iter() {
            let names = resolution
                .skills
                .iter()
                .filter_map(|(name, dir, _)| {
                    let rel = dir.strip_prefix(&mount.folder).ok()?;
                    Some((name.clone(), rel.to_path_buf()))
                })
                .collect();
            roots.push((mount.folder.clone(), names));
        }
        roots.retain(|(_, names)| !names.is_empty());
        let asks: Vec<(PathBuf, Need)> = roots
            .iter()
            .map(|(root, _)| (root.clone(), Need::Advisory))
            .collect();
        let canonicals = self.io.canonical_all(io, &asks);
        // ADR-0355: one place per canonical identity — the path key, so two spellings of one folder are one place.
        let mut places: BTreeMap<String, HashMap<PathKey, PathBuf>> = BTreeMap::new();
        for (at, names) in roots {
            match canonicals.get(&path_key(&at)) {
                Some(Ok(canonical)) => {
                    for (name, rel) in names {
                        // Rebuilt from its components: a `SKILL.md` directly in a root has an empty folder, and
                        //   `join("")` adds a trailing separator that `path_key` keeps.
                        let place: PathBuf = canonical.join(rel).components().collect();
                        places
                            .entry(name)
                            .or_default()
                            .entry(path_key(&place))
                            .or_insert(place);
                    }
                }
                other => tracing::warn!(
                    root = %escape_controls(&at.to_string_lossy()),
                    "skill root canonicalize failed; left out of the same-name skill check: {other:?}"
                ),
            }
        }
        places
            .into_iter()
            .filter(|(_, at)| at.len() > 1)
            .map(|(name, at)| {
                let mut at: Vec<String> = at
                    .values()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                at.sort();
                (name, at)
            })
            .collect()
    }

    /// The backend's own skill roots found from the canonical `cwd` (advisory): its fixed roots, and its per-folder
    /// roots in `cwd` and each parent up to the project root (the nearest folder holding a project marker; none =
    /// `cwd` only); each root that lists is read through the port's `root_skills`. A root that did not answer, or one
    /// of whose reads did not, is left out (logged — B24). No home folder = none looked at.
    fn native_roots(
        &self,
        io: &LaunchIo,
        check: &dyn SettingsCheck,
        mode: AgentOutputFormat,
        profile: &AgentProfile,
        cwd: &Path,
    ) -> Vec<(PathBuf, Vec<(String, PathBuf)>)> {
        let Some(home) = self.values.home.as_deref() else {
            tracing::warn!("no home path; skipping native_skill_roots");
            return Vec::new();
        };
        let env = |name: &str| {
            profile
                .env
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .or_else(|| (self.values.env)(name))
        };
        let places = NativePlaces {
            home,
            cwd,
            env: &env,
        };
        let native = check.native_skill_roots(&places, mode);
        let ancestors: Vec<&Path> = cwd.ancestors().collect();
        let markers: Vec<(PathBuf, Need)> = ancestors
            .iter()
            .flat_map(|folder| {
                native
                    .project_markers
                    .iter()
                    .map(|marker| (folder.join(marker), Need::Advisory))
            })
            .collect();
        let found = self.io.stat_all(io, &markers);
        let project = ancestors
            .iter()
            .position(|folder| {
                native.project_markers.iter().any(|marker| {
                    matches!(
                        found.get(&path_key(&folder.join(marker))),
                        Some(FolderStatus::Ok | FolderStatus::NotADirectory)
                    )
                })
            })
            .unwrap_or(0);
        let candidates: Vec<PathBuf> = native
            .fixed
            .iter()
            .cloned()
            .chain(
                ancestors[..=project]
                    .iter()
                    .flat_map(|folder| native.per_folder.iter().map(|rel| folder.join(rel))),
            )
            .collect();
        let asks: Vec<(PathBuf, Need)> = candidates
            .iter()
            .map(|root| (root.clone(), Need::Advisory))
            .collect();
        let listings = self.io.list_all(io, &asks);
        let mut roots = Vec::new();
        for root in candidates {
            let listing = match listings.get(&path_key(&root)) {
                Some(Ok(listing)) => listing,
                Some(Err(FolderStatus::Unreadable(why))) => {
                    tracing::warn!(
                        root = %escape_controls(&root.to_string_lossy()),
                        "native skill root listing failed; left out of the same-name skill check: {}",
                        escape_controls(&why)
                    );
                    continue;
                }
                _ => continue,
            };
            let reads = FolderReads::new(self, io, &root, Need::Advisory);
            let set = check.root_skills(&listing.entries, &|rel| reads.read(rel), &YamlFrontMatter);
            if let Some(why) = reads.unanswered.into_inner() {
                tracing::warn!(
                    root = %escape_controls(&root.to_string_lossy()),
                        "native skill root file read TimedOut; left out of the same-name skill check: {}",
                    escape_controls(&why)
                );
                continue;
            }
            let names = set
                .skill_names
                .into_iter()
                .map(|skill| (skill.name, skill_folder(&skill.path)))
                .collect();
            roots.push((root, names));
        }
        roots
    }

    /// One stamped file read again for `prepare`, as its stamp says: `(SHA-256, text)`. Gone (not found, not a file)
    /// = `Stale`; a folder file with no answer = `Failed` (`FOLDER_TIMEOUT`); any other error = `Failed`.
    fn reread(&self, io: &LaunchIo, entry: &StampEntry) -> Result<(String, String), PrepareError> {
        let file = Path::new(&entry.file);
        let gone = |e: &io::Error| {
            matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::IsADirectory
            )
        };
        let stale = |e: io::Error| {
            PrepareError::Stale(format!("gone since the check: {}: {e}", entry.file))
        };
        let failed = |e: io::Error| {
            PrepareError::Failed(format!("could not read {} again: {e}", entry.file))
        };
        match &entry.folder {
            None => match self.store.read(file, LIBRARY_READ_CAP) {
                Ok(bytes) => Ok((
                    sha256_hex(&bytes),
                    String::from_utf8_lossy(&bytes).into_owned(),
                )),
                Err(e) if gone(&e) => Err(stale(e)),
                Err(e) => Err(failed(e)),
            },
            Some(folder) => match self.io.read_one(io, file, Need::Required, true) {
                Ok(text) => Ok((sha256_hex(text.as_bytes()), text)),
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    Err(PrepareError::Failed(format!("{FOLDER_TIMEOUT}: {folder}")))
                }
                Err(e) if gone(&e) => Err(stale(e)),
                Err(e) => Err(failed(e)),
            },
        }
    }
}

impl Overlay {
    /// The read behind the item at disk path `file` (a skill: its folder).
    fn read_of(&self, file: &Path) -> Option<&FileRead> {
        self.files.get(&path_key(file))
    }
}

/// The notices of one folder entry's listing: entries left out — the listing's skips, then `unreadable` (files the port
/// could not read) — and a listing cut at its cap. Neither blocks.
// ADR-0363
// ADR-0387
fn listing_notices(folder: &Path, listing: &FolderListing, unreadable: &[String]) -> Vec<Notice> {
    let folder = folder.to_string_lossy();
    let mut notices = Vec::new();
    let skipped: Vec<&String> = listing.skipped.iter().chain(unreadable).collect();
    if !skipped.is_empty() {
        let args = json!({ "folder": folder, "paths": skipped });
        notices.push(notice(FOLDER_LISTING_SKIPPED, args));
    }
    if listing.truncated {
        notices.push(notice(FOLDER_LISTING_PARTIAL, json!({ "folder": folder })));
    }
    notices
}

fn notice(code: &str, args: Value) -> Notice {
    Reason {
        code: code.to_owned(),
        args,
        line: None,
    }
}

/// The folder of a `SKILL.md` given relative to its root.
fn skill_folder(skill_md: &Path) -> PathBuf {
    skill_md.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// What one removal walk did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub folders_removed: usize,
    pub links_detached: usize,
    /// What could not be removed, with the error — left in place.
    pub left: Vec<(PathBuf, String)>,
}

/// Removes every launch's volatile folder under `launch_root` (module rules), keeping `launch_root` itself. A missing
/// `launch_root` is an empty report. The daemon calls it at boot, before the service serves any launch.
pub fn sweep_launch_dir(launch_root: &Path) -> SweepReport {
    let mut report = SweepReport::default();
    // The root gets the per-node rule too: a link is never enumerated — its target's entries are not launch folders.
    let entries = match StdLaunchFs.is_link(launch_root) {
        Ok(false) => fs::read_dir(launch_root),
        Ok(true) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the launch root is a link — not swept",
        )),
        Err(e) => Err(e),
    };
    match entries {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => remove_tree(&entry.path(), &mut report),
                    Err(e) => report.left.push((launch_root.to_path_buf(), e.to_string())),
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => report.left.push((launch_root.to_path_buf(), e.to_string())),
    }
    log_left("sweep_launch_dir", &report);
    report
}

/// Removes launch `(id, epoch)`'s volatile folder (module rules) — only that one; idempotent.
pub(crate) fn release_dir(launch_root: &Path, id: AgentId, epoch: u32) -> SweepReport {
    let mut report = SweepReport::default();
    remove_tree(&volatile_dir(launch_root, id, epoch), &mut report);
    log_left("release", &report);
    report
}

// Recursion goes as deep as the volatile folder: `prepare` and the backend's materialize write shallow trees, links
//   are detached and never entered, and whatever the agent process left below is walked as deep as it goes.
fn remove_tree(path: &Path, report: &mut SweepReport) {
    match StdLaunchFs.is_link(path) {
        Ok(true) => match unlink_dir(path) {
            Ok(()) => report.links_detached += 1,
            Err(e) => report.left.push((path.to_path_buf(), e.to_string())),
        },
        Ok(false) => {
            let removed = match fs::symlink_metadata(path) {
                Ok(meta) if meta.is_dir() => {
                    match fs::read_dir(path) {
                        Ok(entries) => {
                            for entry in entries {
                                match entry {
                                    Ok(entry) => remove_tree(&entry.path(), report),
                                    Err(e) => report.left.push((path.to_path_buf(), e.to_string())),
                                }
                            }
                        }
                        Err(e) => report.left.push((path.to_path_buf(), e.to_string())),
                    }
                    fs::remove_dir(path).map(|()| report.folders_removed += 1)
                }
                Ok(_) => fs::remove_file(path),
                Err(e) => Err(e),
            };
            if let Err(e) = removed {
                if e.kind() != io::ErrorKind::NotFound {
                    report.left.push((path.to_path_buf(), e.to_string()));
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => report.left.push((path.to_path_buf(), e.to_string())),
    }
}

fn log_left(what: &str, report: &SweepReport) {
    if let Some((path, error)) = report.left.first() {
        tracing::warn!(
            op = what,
            path = %escape_controls(&path.to_string_lossy()),
            count = report.left.len(),
            "entries that failed to delete were left: {}",
            escape_controls(error)
        );
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_platform::fs::link_dir;

    use super::*;
    use crate::mounts::MountPath;

    #[test]
    fn merge_instructions_puts_the_priming_first_and_heads_each_source_with_its_path() {
        let merged = merge_instructions(
            Some("\u{feff}priming\r\n\r\n"),
            &[("rules/a.md", "a\n"), ("folder:D:\\r", "\u{feff}b")],
        );
        assert_eq!(
            merged,
            "priming\n\n<!-- rules/a.md -->\na\n\n<!-- folder:D:\\r -->\nb\n"
        );
        assert_eq!(merge_instructions(None, &[]), "");
        assert_eq!(merge_instructions(Some("p"), &[]), "p\n");
        assert_eq!(
            merge_instructions(None, &[("a.md", "a")]),
            "<!-- a.md -->\na\n"
        );
    }

    #[test]
    fn the_hash_is_the_lowercase_hex_sha256_of_the_bytes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A volatile folder with a link at depth 1 and depth 3 into a library skill folder, and plain files.
    fn volatile_with_links(root: &Path) -> (PathBuf, PathBuf) {
        let skill = root.join("lib").join("skills").join("s");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "skill").unwrap();
        let id = AgentId::from_u128(7);
        let dir = volatile_dir(&root.join("launch"), id, 3);
        let deep = dir.join("engram").join("skills");
        fs::create_dir_all(&deep).unwrap();
        fs::write(dir.join(INSTRUCTIONS_FILE), "merged").unwrap();
        fs::write(deep.join("note.txt"), "x").unwrap();
        link_dir(&skill, &dir.join("one")).unwrap();
        link_dir(&skill, &deep.join("three")).unwrap();
        (dir, skill)
    }

    #[test]
    fn release_detaches_links_at_any_depth_keeps_the_library_skill_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let (dir, skill) = volatile_with_links(temp.path());
        let launch_root = temp.path().join("launch");
        let other = volatile_dir(&launch_root, AgentId::from_u128(7), 4);
        fs::create_dir_all(&other).unwrap();

        let report = release_dir(&launch_root, AgentId::from_u128(7), 3);
        assert_eq!(report.links_detached, 2);
        assert_eq!(
            report.folders_removed, 3,
            "the volatile folder, engram, skills"
        );
        assert!(report.left.is_empty(), "{:?}", report.left);
        assert!(!dir.exists());
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), "skill");
        assert!(other.exists(), "only that launch's folder");

        assert_eq!(
            release_dir(&launch_root, AgentId::from_u128(7), 3),
            SweepReport::default(),
            "a second release finds nothing"
        );
    }

    #[test]
    fn a_launch_root_that_is_a_link_is_reported_and_never_enumerated() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(outside.join("keep")).unwrap();
        fs::write(outside.join("keep").join("file.txt"), "x").unwrap();
        let launch_root = temp.path().join("launch");
        link_dir(&outside, &launch_root).unwrap();

        let report = sweep_launch_dir(&launch_root);
        assert_eq!((report.folders_removed, report.links_detached), (0, 0));
        assert_eq!(
            report.left.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            [&launch_root]
        );
        assert!(outside.join("keep").join("file.txt").exists());
    }

    #[test]
    fn a_release_whose_launch_folder_is_a_link_detaches_it_and_keeps_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("file.txt"), "x").unwrap();
        let launch_root = temp.path().join("launch");
        fs::create_dir_all(&launch_root).unwrap();
        let id = AgentId::from_u128(9);
        link_dir(&outside, &volatile_dir(&launch_root, id, 1)).unwrap();

        let report = release_dir(&launch_root, id, 1);
        assert_eq!((report.folders_removed, report.links_detached), (0, 1));
        assert!(report.left.is_empty(), "{:?}", report.left);
        assert!(outside.join("file.txt").exists());
        assert!(!volatile_dir(&launch_root, id, 1).exists());
    }

    #[test]
    fn the_sweep_removes_every_volatile_folder_keeps_the_root_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let (dir, skill) = volatile_with_links(temp.path());
        let launch_root = temp.path().join("launch");
        let stray = launch_root.join("stray.txt");
        fs::write(&stray, "x").unwrap();

        let report = sweep_launch_dir(&launch_root);
        assert_eq!(report.links_detached, 2);
        assert_eq!(report.folders_removed, 3);
        assert!(report.left.is_empty(), "{:?}", report.left);
        assert!(!dir.exists() && !stray.exists());
        assert!(launch_root.is_dir(), "the launch root stays");
        assert_eq!(fs::read_to_string(skill.join("SKILL.md")).unwrap(), "skill");

        assert_eq!(sweep_launch_dir(&launch_root), SweepReport::default());
        assert_eq!(
            sweep_launch_dir(&temp.path().join("absent")),
            SweepReport::default()
        );
    }

    // ── the launch check: a temp library on the real store, real ports, gate-controlled real folder ops ───────────

    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Condvar, PoisonError};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use engram_dashboard_agent::backend::materialize;
    use engram_dashboard_agent::launch::{
        BUILTIN_UNAVAILABLE, CODEX_SAME_NAME_SKILLS, CODEX_TERMINAL_OMITTED,
    };
    use engram_dashboard_agent::profile::{
        AgentCommand, AttachedPreset, ProfileRegistry, ProfileStore, StoreError,
    };
    use engram_dashboard_base::sync::write;

    use crate::backends::fake::{Registry, Scripted};
    use crate::backends::{AgentBackends, Backends};
    use crate::folder::LIST_CAP;
    use crate::folder::{
        BoundedIo, FolderOps, LaunchWaits, Spawn, StdFolderOps, IO_LIMIT, TIMEOUT,
    };
    use crate::store::{ContentTag, FsStore, LibraryStore, ScanListing, TrashBundle};
    use crate::{LibraryValues, MERGE_JOB, PATH_THROUGH_LINK};

    const JSON: AgentOutputFormat = AgentOutputFormat::StreamJson;
    const TERMINAL: AgentOutputFormat = AgentOutputFormat::Terminal;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Holds every op on a path.
    const EVERY_OP: &str = "*";

    #[derive(Default)]
    struct GatedState {
        /// (op or [`EVERY_OP`], path).
        held: HashSet<(&'static str, PathBuf)>,
        released_all: bool,
        /// Every op as it started.
        calls: Vec<(&'static str, PathBuf)>,
        panics: HashSet<PathBuf>,
        skipped: HashMap<PathBuf, Vec<String>>,
        /// Paths whose stat answers an access error.
        denied: HashSet<PathBuf>,
        /// Files whose read answers an access error.
        unreadable_files: HashSet<PathBuf>,
    }

    type Probe = Box<dyn Fn(&Path) + Send + Sync>;

    /// The real folder ops on the temp folders; a test can hold any path's ops (until it releases them or the world
    /// ends), make a path's op panic, add skipped entries to a listing, and count the ops.
    #[derive(Default)]
    struct Gated {
        state: Mutex<GatedState>,
        moved: Condvar,
        /// Runs as every op starts, with its path.
        probe: Mutex<Option<Probe>>,
    }

    impl Gated {
        fn hold(&self, path: &Path) {
            self.hold_op(EVERY_OP, path);
        }

        fn hold_op(&self, op: &'static str, path: &Path) {
            lock(&self.state).held.insert((op, path.to_path_buf()));
        }

        fn release_op(&self, op: &'static str, path: &Path) {
            lock(&self.state).held.remove(&(op, path.to_path_buf()));
            self.moved.notify_all();
        }

        fn release_all(&self) {
            lock(&self.state).released_all = true;
            self.moved.notify_all();
        }

        fn count(&self, op: &str, path: &Path) -> usize {
            lock(&self.state)
                .calls
                .iter()
                .filter(|(o, p)| *o == op && p == path)
                .count()
        }

        fn ops(&self, op: &str) -> usize {
            lock(&self.state)
                .calls
                .iter()
                .filter(|(o, _)| *o == op)
                .count()
        }

        fn wait_started(&self, op: &str, path: &Path) {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut state = lock(&self.state);
            while !state.calls.iter().any(|(o, p)| *o == op && p == path) {
                let now = Instant::now();
                assert!(now < deadline, "{op} {path:?} never started");
                state = self
                    .moved
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }

        fn enter(&self, op: &'static str, path: &Path) {
            if let Some(probe) = lock(&self.probe).as_ref() {
                probe(path);
            }
            let mut state = lock(&self.state);
            state.calls.push((op, path.to_path_buf()));
            self.moved.notify_all();
            let held = |state: &GatedState, by: &'static str| {
                state.held.contains(&(by, path.to_path_buf()))
            };
            while (held(&state, op) || held(&state, EVERY_OP)) && !state.released_all {
                state = self
                    .moved
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            let panics = state.panics.contains(path);
            drop(state);
            assert!(!panics, "a folder op panics on purpose");
        }
    }

    impl FolderOps for Gated {
        fn stat(&self, p: &Path) -> FolderStatus {
            self.enter("stat", p);
            if lock(&self.state).denied.contains(p) {
                return FolderStatus::Unreadable("access denied".to_owned());
            }
            StdFolderOps.stat(p)
        }

        fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
            self.enter("list", p);
            let mut listing = StdFolderOps.list(p, cap)?;
            if let Some(skipped) = lock(&self.state).skipped.get(p) {
                listing.skipped = skipped.clone();
            }
            Ok(listing)
        }

        fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
            self.enter("canonical", p);
            StdFolderOps.canonical(p)
        }

        fn identity(&self, p: &Path) -> io::Result<engram_dashboard_platform::fs::FileIdentity> {
            self.enter("identity", p);
            StdFolderOps.identity(p)
        }

        fn read(&self, p: &Path, cap: u64) -> io::Result<String> {
            self.enter("read", p);
            if lock(&self.state).unreadable_files.contains(p) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            StdFolderOps.read(p, cap)
        }
    }

    /// What the spawner does with the service's merge jobs; folder workers always get a thread.
    #[derive(Clone, Copy, PartialEq)]
    enum Merges {
        Run,
        /// Kept until the test runs them.
        Defer,
        /// Refused — the job is dropped unrun.
        Fail,
    }

    type Job = Box<dyn FnOnce() + Send>;

    struct TestSpawn {
        merges: Merges,
        running: Mutex<Vec<JoinHandle<()>>>,
        deferred: Mutex<Vec<Job>>,
    }

    impl Spawn for TestSpawn {
        fn spawn(&self, name: &str, job: Job) -> io::Result<()> {
            let merge = name == MERGE_JOB;
            match (merge, self.merges) {
                (true, Merges::Fail) => {
                    drop(job);
                    return Err(io::Error::other("no threads left"));
                }
                (true, Merges::Defer) => {
                    lock(&self.deferred).push(job);
                    return Ok(());
                }
                _ => {}
            }
            let handle = thread::Builder::new().name(name.to_owned()).spawn(job)?;
            if merge {
                lock(&self.running).push(handle);
            }
            Ok(())
        }
    }

    impl TestSpawn {
        fn merges_done(&self) {
            for handle in std::mem::take(&mut *lock(&self.running)) {
                handle.join().expect("a merge job panicked");
            }
        }

        fn take_deferred(&self) -> Vec<Job> {
            std::mem::take(&mut *lock(&self.deferred))
        }
    }

    type StoreProbe = Box<dyn Fn(&Path) + Send + Sync>;

    /// The real store, with every file read logged, a probe on every op and a whole-library scan a test can park.
    #[derive(Default)]
    struct LoggedStore {
        reads: Mutex<Vec<PathBuf>>,
        first_links: AtomicUsize,
        /// Runs as every scan, link probe and read starts, with its path.
        probe: Mutex<Option<StoreProbe>>,
        /// (hold whole scans, a whole scan is parked).
        park: Mutex<(bool, bool)>,
        moved: Condvar,
    }

    impl LoggedStore {
        fn hold_whole_scans(&self, hold: bool) {
            lock(&self.park).0 = hold;
            self.moved.notify_all();
        }

        fn wait_parked(&self) {
            let mut park = lock(&self.park);
            while !park.1 {
                park = self
                    .moved
                    .wait(park)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }

        fn read_any(&self, path: &Path) -> bool {
            lock(&self.reads).iter().any(|read| read == path)
        }

        fn probe(&self, path: &Path) {
            if let Some(probe) = lock(&self.probe).as_ref() {
                probe(path);
            }
        }
    }

    impl LibraryStore for LoggedStore {
        fn scan(
            &self,
            root: &Path,
            under: Option<&MountPath>,
            seq: &dyn Fn() -> u64,
        ) -> io::Result<ScanListing> {
            self.probe(root);
            if under.is_none() {
                let mut park = lock(&self.park);
                if park.0 {
                    park.1 = true;
                    self.moved.notify_all();
                    while park.0 {
                        park = self
                            .moved
                            .wait(park)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    park.1 = false;
                }
            }
            FsStore.scan(root, under, seq)
        }

        fn first_link(&self, root: &Path, rel: &MountPath) -> io::Result<Option<MountPath>> {
            self.probe(root);
            self.first_links.fetch_add(1, Ordering::SeqCst);
            FsStore.first_link(root, rel)
        }

        fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
            self.probe(path);
            lock(&self.reads).push(path.to_path_buf());
            FsStore.read(path, cap)
        }

        fn tag(&self, path: &Path) -> io::Result<ContentTag> {
            FsStore.tag(path)
        }

        fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            FsStore.write_atomic(path, bytes)
        }

        fn create_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            FsStore.create_new(path, bytes)
        }
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            FsStore.create_dir(path)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.rename(from, to)
        }

        fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
            FsStore.copy(from, to)
        }

        fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf> {
            FsStore.trash(root, path, bundle)
        }

        fn exists(&self, path: &Path) -> bool {
            FsStore.exists(path)
        }
    }

    struct NoProfiles;

    impl ProfileStore for NoProfiles {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            Vec::new()
        }
    }

    struct Setup {
        backends: Arc<dyn Backends>,
        waits: LaunchWaits,
        list_cap: usize,
        merges: Merges,
        /// The built-in priming's text; `None` = no built-in folder at all.
        priming: Option<&'static str>,
        /// Whether the library root is set.
        root: bool,
        /// The name `lib\` is mounted under.
        mount: &'static str,
        /// Whether `team\` is mounted too, as `team` (with `root`).
        team: bool,
    }

    impl Default for Setup {
        fn default() -> Self {
            Setup {
                backends: Arc::new(AgentBackends),
                waits: waits(10_000, 2_000),
                list_cap: LIST_CAP,
                merges: Merges::Run,
                priming: Some("PRIMING"),
                root: true,
                mount: "library",
                team: false,
            }
        }
    }

    fn waits(required_ms: u64, advisory_ms: u64) -> LaunchWaits {
        LaunchWaits {
            required: ms(required_ms),
            advisory: ms(advisory_ms),
        }
    }

    /// A temp folder (canonical spelling) with `lib\` (the root), `work\` (the working folder), `home\` and, unless
    /// the setup says otherwise, `builtin\` (the default preset loads the priming).
    struct World {
        _temp: tempfile::TempDir,
        base: PathBuf,
        ops: Arc<Gated>,
        store: Arc<LoggedStore>,
        spawn: Arc<TestSpawn>,
        svc: Arc<LibraryService>,
    }

    impl World {
        fn new() -> World {
            World::with(Setup::default())
        }

        fn with(setup: Setup) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            for folder in ["lib", "team", "work", "home"] {
                fs::create_dir_all(base.join(folder)).unwrap();
            }
            if let Some(priming) = setup.priming {
                put(&base, "builtin/instructions/priming.md", priming);
                put(
                    &base,
                    "builtin/presets/default.yaml",
                    "kind: preset\nversion: 1\ninstructions:\n  - engram/priming\n",
                );
            }
            let ops = Arc::new(Gated::default());
            let store = Arc::new(LoggedStore::default());
            let spawn = Arc::new(TestSpawn {
                merges: setup.merges,
                running: Mutex::default(),
                deferred: Mutex::default(),
            });
            let values = LibraryValues {
                registry: Arc::new(ProfileRegistry::new(Arc::new(NoProfiles))),
                backends: setup.backends,
                config: match setup.root {
                    true => crate::config::LibraryConfig::mounted(
                        &base.join("state"),
                        [(setup.mount, "lib", true), ("team", "team", setup.team)]
                            .into_iter()
                            .filter(|(_, _, mounted)| *mounted)
                            .map(|(name, folder, _)| crate::config::MountSetting {
                                name: crate::mounts::MountName::parse(name).unwrap(),
                                place: crate::config::MountPlace::Explicit(base.join(folder)),
                            })
                            .collect(),
                    ),
                    false => crate::config::LibraryConfig::missing(&base.join("state")),
                },
                default_root: base.join("default-library"),
                daemon_id: "test-daemon".to_owned(),
                root_generation_base: 7,
                builtin_dir: setup.priming.map(|_| base.join("builtin")),
                launch_root: base.join("launch"),
                home: Some(base.join("home")),
                env: Arc::new(|_| None),
            };
            let io = BoundedIo::new(
                IO_LIMIT,
                setup.waits,
                setup.list_cap,
                Arc::clone(&ops) as Arc<dyn FolderOps>,
                Arc::clone(&spawn) as Arc<dyn Spawn>,
            );
            let store_port = Arc::clone(&store) as Arc<dyn LibraryStore>;
            let svc = Arc::new(LibraryService::new(values, store_port, io));
            World {
                _temp: temp,
                base,
                ops,
                store,
                spawn,
                svc,
            }
        }

        fn at(&self, rel: &str) -> PathBuf {
            at(&self.base, rel)
        }

        fn put(&self, rel: &str, text: &str) -> PathBuf {
            put(&self.base, rel, text)
        }

        fn has(&self, rel: &str) -> bool {
            let path = LibPath::parse(rel).unwrap();
            self.svc
                .with_read(|state| state.catalog.get(&key(&path)).is_some())
        }

        /// A folder entry `rel` holding `files` (relative path, text).
        fn folder(&self, rel: &str, files: &[(&str, &str)]) -> PathBuf {
            let folder = self.at(rel);
            fs::create_dir_all(&folder).unwrap();
            for (file, text) in files {
                self.put(&format!("{rel}/{file}"), text);
            }
            folder
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            self.ops.release_all();
            self.store.hold_whole_scans(false);
        }
    }

    fn at(base: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(base.to_path_buf(), |path, part| path.join(part))
    }

    fn put(base: &Path, rel: &str, text: &str) -> PathBuf {
        let path = at(base, rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn preset(
        instructions: &[&str],
        skills: &[&str],
        settings: &[&str],
        folders: &[&Path],
    ) -> String {
        let mut text = "kind: preset\nversion: 1\n".to_owned();
        for (list, items) in [
            ("instructions", instructions),
            ("skills", skills),
            ("settings", settings),
        ] {
            if !items.is_empty() {
                text += &format!("{list}:\n");
                for item in items {
                    text += &format!("  - {item}\n");
                }
            }
        }
        if !folders.is_empty() {
            text += "folders:\n";
            for folder in folders {
                text += &format!("  - '{}'\n", folder.display());
            }
        }
        text
    }

    // ADR-0400 decision 2: a valid mount name shaped like a temp file does not make its items absent to a launch.
    #[test]
    fn a_launch_reads_the_items_of_a_mount_named_like_a_temp_file() {
        let w = World::with(Setup {
            mount: "work.tmp1.2",
            ..Setup::default()
        });
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(&["work.tmp1.2/rules/a.md"], &[], &[], &[]),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["work.tmp1.2/P.yaml"], &w.at("work")))
            .unwrap();
        let sources: Vec<&str> = plan
            .instructions
            .iter()
            .map(|source| source.path.as_str())
            .collect();
        assert_eq!(sources, ["work.tmp1.2/rules/a.md"]);
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
    }

    // ── two mounts (ADR-0400 decision 19) ──────────────────────────────────────────────────────────────────────

    fn two_mounts() -> World {
        World::with(Setup {
            team: true,
            ..Setup::default()
        })
    }

    fn sources(plan: &LaunchPlan) -> Vec<(&str, PathBuf)> {
        plan.instructions
            .iter()
            .map(|source| (source.path.as_str(), source.file.clone()))
            .collect()
    }

    /// A preset in one mount reaches items in another: each file is read fresh in its own mount's folder, stamped there,
    /// and a reference at or below a link inside the other mount is probed in that mount's folder.
    #[test]
    fn a_launch_reads_each_mounts_files_in_its_own_folder_and_probes_its_links_there() {
        let w = two_mounts();
        w.put("team/rules/t.md", "T");
        w.put("team/skills/gamma/SKILL.md", &skill("gamma"));
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(
                &["team/rules/t.md", "library/rules/a.md"],
                &["team/skills/gamma"],
                &[],
                &[],
            ),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            sources(&plan),
            [
                ("team/rules/t.md", w.at("team/rules/t.md")),
                ("library/rules/a.md", w.at("lib/rules/a.md")),
            ]
        );
        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.skills[0].dir, w.at("team/skills/gamma"));
        let mut stamped = vec![
            lossy(&w.at("lib/P.yaml")),
            lossy(&w.at("lib/rules/a.md")),
            lossy(&w.at("team/rules/t.md")),
            lossy(&w.at("team/skills/gamma/SKILL.md")),
        ];
        stamped.sort();
        assert_eq!(stamp_files(&plan), stamped);
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
        assert_eq!(
            w.ops.count("stat", &w.at("team")),
            1,
            "one stat per mount per launch"
        );

        let outside = w.folder("outside", &[("x.md", "X")]);
        link_dir(&outside, &w.at("team/linked")).unwrap();
        w.put("lib/P.yaml", &preset(&["team/linked/x.md"], &[], &[], &[]));
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        assert_eq!(codes(&blocked), [("broken", PATH_THROUGH_LINK)]);
        assert!(!w.store.read_any(&outside.join("x.md")));
    }

    /// A mount that is not there leaves out only its own items, told once with `mount`; the other mount's items load,
    /// and its own missing item keeps its own notice. Nothing in the absent mount is read or probed.
    #[test]
    fn an_absent_mount_skips_only_its_own_items_with_one_notice_and_the_other_mount_launches() {
        let w = two_mounts();
        fs::remove_dir_all(w.at("team")).unwrap();
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(
                &[
                    "library/rules/a.md",
                    "team/a.md",
                    "team/b.md",
                    "library/gone.md",
                ],
                &["team/skills/x"],
                &[],
                &[],
            ),
        );
        let plan = w
            .svc
            .check(&agent(
                codex(JSON),
                &["library/P.yaml", "team/T.yaml"],
                &w.at("work"),
            ))
            .unwrap();
        assert_eq!(
            sources(&plan),
            [("library/rules/a.md", w.at("lib/rules/a.md"))]
        );
        assert_eq!(
            plan.notices,
            [
                notice(
                    LIBRARY_UNAVAILABLE_SKIPPED,
                    json!({
                        "mount": "team",
                        "message": "missing",
                        "items": ["team/a.md", "team/b.md", "team/skills/x", "team/T.yaml"],
                    })
                ),
                skipped_item("library/gone.md", &["library/P.yaml"], Some(7)),
            ]
        );
        assert_eq!(w.ops.count("stat", &w.at("team")), 1);
        assert_eq!(
            w.store.first_links.load(Ordering::SeqCst),
            1,
            "the other mount's not-found row only"
        );
        assert!(!w.store.read_any(&w.at("team/T.yaml")));
    }

    /// A mount that exists but cannot be read is its ④ row naming it, beside the rows the other mounts give — the
    /// launch is refused before any folder IO or advisory step, and nothing in that mount is read.
    #[test]
    fn an_unreadable_mount_blocks_with_its_row_beside_the_other_mounts_rows_and_no_folder_io() {
        let w = two_mounts();
        lock(&w.ops.state).denied.insert(w.at("team"));
        let folder = w.folder("rules", &[]);
        w.put("team/T.yaml", &preset(&[], &[], &[], &[]));
        w.put("lib/bad.yaml", "kind: preset\nversion: [\n");
        w.put("lib/P.yaml", &preset(&["team/a.md"], &[], &[], &[&folder]));
        // ADR-0400 decision 10: the load-time judgement canonicalizes the mount folders first; not the check's IO.
        w.svc.judge_at_load(&w.svc.io.begin_read());
        let judged = w.ops.ops("canonical");
        let blocked = w
            .svc
            .check(&agent(
                codex(JSON),
                &["library/P.yaml", "library/bad.yaml", "team/T.yaml"],
                &w.at("work"),
            ))
            .unwrap_err();
        assert_eq!(
            blocked.reasons,
            [
                BlockReason::Broken {
                    item: "library/bad.yaml".to_owned(),
                    via: Vec::new(),
                    reason: match &blocked.reasons[0] {
                        BlockReason::Broken { reason, .. } if reason.code == "yaml-syntax" => {
                            reason.clone()
                        }
                        other => panic!("{other:?}"),
                    },
                },
                BlockReason::LibraryUnavailable {
                    reason: Reason {
                        code: crate::LIBRARY_UNAVAILABLE.to_owned(),
                        args: json!({ "mount": "team", "message": "access denied" }),
                        line: None,
                    }
                },
            ]
        );
        assert_eq!(w.ops.count("stat", &folder), 0, "no folder IO");
        assert_eq!(w.ops.ops("canonical"), judged, "no advisory phase");
        assert!(!w.store.read_any(&w.at("team/T.yaml")));
    }

    /// A mount first reached through a nested reference — the second wave of stats — is judged by its own stat as one
    /// an agent attaches directly: one that is not there skips its items with one notice, one that cannot be read
    /// blocks with its row; each mount is stat'ed once.
    #[test]
    fn a_mount_first_reached_in_a_later_wave_is_judged_by_its_own_stat() {
        for absent in [true, false] {
            let w = two_mounts();
            match absent {
                true => fs::remove_dir_all(w.at("team")).unwrap(),
                false => {
                    lock(&w.ops.state).denied.insert(w.at("team"));
                }
            }
            w.put("lib/rules/a.md", "A");
            w.put(
                "lib/P.yaml",
                &preset(&["library/rules/a.md", "team/t.md"], &[], &[], &[]),
            );
            let checked = w
                .svc
                .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")));
            match absent {
                true => {
                    let plan = checked.unwrap();
                    assert_eq!(
                        sources(&plan),
                        [("library/rules/a.md", w.at("lib/rules/a.md"))]
                    );
                    assert_eq!(
                        plan.notices,
                        [notice(
                            LIBRARY_UNAVAILABLE_SKIPPED,
                            json!({ "mount": "team", "message": "missing", "items": ["team/t.md"] })
                        )]
                    );
                }
                false => assert_eq!(
                    checked.unwrap_err().reasons,
                    [BlockReason::LibraryUnavailable {
                        reason: Reason {
                            code: crate::LIBRARY_UNAVAILABLE.to_owned(),
                            args: json!({ "mount": "team", "message": "access denied" }),
                            line: None,
                        }
                    }]
                ),
            }
            assert_eq!(
                (
                    w.ops.count("stat", &w.at("lib")),
                    w.ops.count("stat", &w.at("team"))
                ),
                (1, 1),
                "absent: {absent}"
            );
        }
    }

    /// A first name that names no mount, beside a healthy mount, is skipped as an unregistered mount — one notice with
    /// its items — and the healthy mount's items load.
    #[test]
    fn an_unregistered_name_beside_a_healthy_mount_skips_only_its_items() {
        let w = two_mounts();
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/a.md", "nowhere/x.md"], &[], &[], &[]),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            sources(&plan),
            [("library/rules/a.md", w.at("lib/rules/a.md"))]
        );
        assert_eq!(
            plan.notices,
            [notice(
                LIBRARY_UNAVAILABLE_SKIPPED,
                json!({
                    "mount": "nowhere",
                    "message": crate::resolve::NOT_REGISTERED,
                    "items": ["nowhere/x.md"],
                })
            )]
        );
        assert_eq!(
            w.ops.count("stat", &w.at("team")),
            0,
            "no reference reaches team"
        );
    }

    /// The overlay follows references by the resolution's tag rule: a mount only another backend's entry reaches is
    /// never stat'ed — its dead share neither spends the launch's required budget nor leaves the folder entries
    /// unchecked — and nothing in it is read (ADR-0400 decision 19).
    #[test]
    fn a_mount_only_another_backends_entry_reaches_is_not_stat_ed_by_a_launch() {
        let w = World::with(Setup {
            team: true,
            waits: waits(500, 300),
            ..Setup::default()
        });
        w.ops.hold_op("stat", &w.at("team"));
        let folder = w.folder("rules", &[]);
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &format!(
                "kind: preset\nversion: 1\ninstructions:\n  - library/rules/a.md\n  - {{path: team/a.md, backend: \
                 claude}}\nskills:\n  - {{path: team/skills/x, backend: claude}}\nfolders:\n  - '{}'\n",
                folder.display()
            ),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            sources(&plan),
            [("library/rules/a.md", w.at("lib/rules/a.md"))]
        );
        assert_eq!(plan.folders.len(), 1, "the folder entry was checked");
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
        assert_eq!(w.ops.count("stat", &w.at("team")), 0);
        assert!(!w.store.read_any(&w.at("team/a.md")));
    }

    /// The same-name skill check takes a loaded library skill at its own mount's folder: only a mount with a loaded
    /// skill is canonicalized.
    #[test]
    fn the_same_name_check_places_a_skill_in_its_own_mounts_folder() {
        let w = two_mounts();
        fs::create_dir_all(w.at("proj/.git")).unwrap();
        w.put("proj/.agents/skills/gamma/SKILL.md", &skill("gamma"));
        let cwd = w.folder("proj/sub", &[]);
        w.put("team/skills/gamma/SKILL.md", &skill("gamma"));
        w.put("lib/P.yaml", &preset(&[], &["team/skills/gamma"], &[], &[]));
        // ADR-0400 decision 10: the load-time judgement canonicalizes each mount folder once first; not the check's.
        w.svc.judge_at_load(&w.svc.io.begin_read());
        let (team, lib) = (
            w.ops.count("canonical", &w.at("team")),
            w.ops.count("canonical", &w.at("lib")),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &cwd))
            .unwrap();
        let mut paths = vec![
            lossy(&w.at("team/skills/gamma")),
            lossy(&w.at("proj/.agents/skills/gamma")),
        ];
        paths.sort();
        assert_eq!(
            plan.notices,
            [notice(
                CODEX_SAME_NAME_SKILLS,
                json!({ "name": "gamma", "paths": paths })
            )]
        );
        assert_eq!(w.ops.count("canonical", &w.at("team")) - team, 1);
        assert_eq!(w.ops.count("canonical", &w.at("lib")) - lib, 0);
    }

    fn skill(name: &str) -> String {
        format!("---\nname: {name}\ndescription: does {name}\n---\nbody\n")
    }

    fn codex(mode: AgentOutputFormat) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: Vec::new(),
            output_format: mode,
        }
    }

    fn claude(mode: AgentOutputFormat) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: Vec::new(),
            output_format: mode,
        }
    }

    /// An agent attaching `presets` (and not the built-in default preset) in `cwd`.
    fn agent(command: AgentCommand, presets: &[&str], cwd: &Path) -> AgentProfile {
        let mut profile = AgentProfile::new(
            "a".to_owned(),
            command,
            cwd.to_path_buf(),
            Vec::new(),
            false,
        );
        profile.default_preset = false;
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    fn with_default(mut profile: AgentProfile) -> AgentProfile {
        profile.default_preset = true;
        profile
    }

    fn code(reason: &BlockReason) -> (&'static str, &str) {
        match reason {
            BlockReason::Missing { reason, .. } => ("missing", reason.code.as_str()),
            BlockReason::Broken { reason, .. } => ("broken", reason.code.as_str()),
            BlockReason::Conflict { reason, .. } => ("conflict", reason.code.as_str()),
            BlockReason::LibraryUnavailable { reason } => ("library", reason.code.as_str()),
        }
    }

    fn codes(blocked: &LaunchBlocked) -> Vec<(&'static str, &str)> {
        blocked.reasons.iter().map(code).collect()
    }

    fn stamp_files(plan: &LaunchPlan) -> Vec<String> {
        let stamp: Stamp = serde_json::from_str(plan.stamp.as_deref().unwrap()).unwrap();
        let mut files: Vec<String> = stamp.files.into_iter().map(|entry| entry.file).collect();
        files.sort();
        files
    }

    fn lossy(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// The (required, advisory) time left to the launch `scope` holds.
    fn left(scope: &LaunchScope) -> (Duration, Duration) {
        scope
            .get::<Launch>()
            .expect("this library's scope")
            .io
            .left()
    }

    /// Waits for one signal (fails after 20 s instead of hanging).
    fn ack(rx: &mpsc::Receiver<()>) {
        rx.recv_timeout(Duration::from_secs(20))
            .expect("the acknowledgement never came");
    }

    // ── passing and blocking ────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_passing_check_carries_the_library_and_folder_materials_and_stamps_each_source() {
        let w = World::new();
        let folder = w.folder(
            "rules",
            &[
                ("AGENTS.md", "folder rules"),
                (".agents/skills/beta/SKILL.md", &skill("beta")),
            ],
        );
        w.put("lib/rules/a.md", "A");
        w.put("lib/skills/alpha/SKILL.md", &skill("alpha"));
        w.put(
            "lib/settings/s.yaml",
            "kind: settings\nversion: 1\ncodex:\n  - model=o3\n",
        );
        w.put(
            "lib/P.yaml",
            &preset(
                &["library/rules/a.md"],
                &["library/skills/alpha"],
                &["library/settings/s.yaml"],
                &[&folder],
            ),
        );
        let profile = with_default(agent(codex(JSON), &["library/P.yaml"], &w.at("work")));

        let plan = w.svc.check(&profile).unwrap();
        assert!(plan.real);
        assert_eq!(plan.backend, "codex");
        let sources: Vec<(&str, &Path)> = plan
            .instructions
            .iter()
            .map(|source| (source.path.as_str(), source.file.as_path()))
            .collect();
        let folder_source = folder_item(&folder);
        let agents_md = folder.join("AGENTS.md");
        let a_md = w.at("lib/rules/a.md");
        assert_eq!(
            sources,
            [
                ("library/rules/a.md", a_md.as_path()),
                (folder_source.as_str(), agents_md.as_path())
            ]
        );
        assert_eq!(
            plan.skills,
            [SkillRef {
                name: "alpha".to_owned(),
                dir: w.at("lib/skills/alpha")
            }]
        );
        assert_eq!(plan.raw_settings.len(), 1);
        assert_eq!(plan.raw_settings[0].preset, "library/P.yaml");
        assert!(plan.include_priming);
        assert_eq!(plan.folders, [folder.clone()]);
        assert_eq!(
            plan.extra_skill_roots,
            [folder.join(".agents").join("skills")]
        );
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
        let mut stamped = vec![
            lossy(&w.at("lib/P.yaml")),
            lossy(&w.at("lib/settings/s.yaml")),
            lossy(&a_md),
            lossy(&w.at("lib/skills/alpha/SKILL.md")),
            lossy(&agents_md),
        ];
        stamped.sort();
        assert_eq!(
            stamp_files(&plan),
            stamped,
            "native and preview-only files are not stamped"
        );

        w.spawn.merges_done();
        assert!(
            w.has("library/P.yaml") && w.has("library/rules/a.md") && w.has("library/skills/alpha")
        );
    }

    /// A missing item beside broken and conflicting ones: the refusal holds only the blocking rows, in kind order
    /// (ADR-0387).
    #[test]
    fn a_check_blocks_on_broken_and_conflicting_items_and_never_on_a_missing_one() {
        let w = World::new();
        w.put("lib/skills/a/SKILL.md", &skill("dup"));
        w.put("lib/skills/b/SKILL.md", &skill("dup"));
        w.put("lib/bad.yaml", "kind: settings\nversion: [\n");
        w.put(
            "lib/P.yaml",
            &preset(
                &["library/gone.md"],
                &["library/skills/a", "library/skills/b"],
                &["library/bad.yaml"],
                &[],
            ),
        );
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        assert_eq!(
            codes(&blocked),
            [
                ("broken", "yaml-syntax"),
                ("conflict", "skill-name-conflict")
            ]
        );
    }

    /// The `missing-item-skipped` notice of a skipped row.
    fn skipped_item(item: &str, via: &[&str], line: Option<u32>) -> Notice {
        Reason {
            code: MISSING_ITEM_SKIPPED.to_owned(),
            args: json!({ "item": item, "via": via }),
            line,
        }
    }

    /// Every kind of missing item — an attached preset, a nested preset, an instruction, a skill, a settings file and
    /// a folder entry — is left out of the materials and named in one `missing-item-skipped` notice each, on claude
    /// and codex JSON; the folders the plan carries drop exactly the skipped folder (ADR-0387).
    #[test]
    fn missing_items_of_every_kind_are_left_out_and_each_named_once() {
        for command in [claude(JSON), codex(JSON)] {
            let w = World::new();
            let kept = w.folder("kept", &[]);
            let gone = w.at("gone");
            w.put("lib/rules/a.md", "A");
            w.put(
                "lib/P.yaml",
                &format!(
                    "{}presets:\n  - library/nested.yaml\n",
                    preset(
                        &["library/rules/a.md", "library/rules/gone.md"],
                        &["library/skills/gone"],
                        &["library/settings/gone.yaml"],
                        &[&kept, &gone],
                    )
                ),
            );
            let profile = agent(
                command.clone(),
                &["library/P.yaml", "library/absent.yaml"],
                &w.at("work"),
            );
            let plan = w.svc.check(&profile).unwrap();
            let sources: Vec<&str> = plan.instructions.iter().map(|s| s.path.as_str()).collect();
            assert_eq!(sources, ["library/rules/a.md"], "{command:?}");
            assert!(plan.skills.is_empty() && plan.raw_settings.is_empty());
            assert_eq!(plan.folders, [kept.clone()], "{command:?}");
            let gone_item = folder_item(&gone);
            assert_eq!(
                plan.notices,
                [
                    skipped_item("library/nested.yaml", &["library/P.yaml"], Some(14)),
                    skipped_item("library/rules/gone.md", &["library/P.yaml"], Some(5)),
                    skipped_item("library/settings/gone.yaml", &["library/P.yaml"], Some(9)),
                    skipped_item("library/skills/gone", &["library/P.yaml"], Some(7)),
                    skipped_item("library/absent.yaml", &[], None),
                    skipped_item(&gone_item, &["library/P.yaml"], Some(12)),
                ],
                "{command:?}"
            );
        }
    }

    #[test]
    fn a_codex_json_skill_with_an_empty_description_blocks_the_check() {
        let w = World::new();
        w.put(
            "lib/skills/e/SKILL.md",
            "---\nname: e\ndescription: ''\n---\nbody\n",
        );
        w.put("lib/P.yaml", &preset(&[], &["library/skills/e"], &[], &[]));
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        assert!(
            matches!(&blocked.reasons[..], [BlockReason::Broken { item, .. }] if item == "library/skills/e"),
            "{blocked:?}"
        );
    }

    /// A mount that exists but cannot be read — an access error, or a stat held past the budget — is block ④ naming it,
    /// here the whole answer: no folder IO, no advisory phase, no working-folder row, no library file read (ADR-0361 ·
    /// ADR-0387 · ADR-0400 decision 19).
    #[test]
    fn block_four_is_the_whole_answer_and_asks_nothing_else() {
        for held in [false, true] {
            // A codex terminal agent whose working folder the terminal line cannot carry would add a ② row.
            let w = World::with(Setup {
                waits: waits(500, 300),
                ..Setup::default()
            });
            let unsafe_cwd = w.at("a&b");
            fs::create_dir_all(&unsafe_cwd).unwrap();
            let folder = w.folder("rules", &[]);
            w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
            match held {
                true => w.ops.hold_op("stat", &w.at("lib")),
                false => {
                    lock(&w.ops.state).denied.insert(w.at("lib"));
                }
            }
            let profile = with_default(agent(codex(TERMINAL), &["library/P.yaml"], &unsafe_cwd));
            let blocked = w.svc.check(&profile).unwrap_err();
            let why = if held { TIMEOUT } else { "access denied" };
            assert_eq!(
                blocked.reasons,
                [BlockReason::LibraryUnavailable {
                    reason: Reason {
                        code: crate::LIBRARY_UNAVAILABLE.to_owned(),
                        args: json!({ "mount": "library", "message": why }),
                        line: None,
                    }
                }],
                "held: {held}"
            );
            assert_eq!(w.ops.count("stat", &w.at("lib")), 1);
            assert_eq!(w.ops.count("stat", &folder), 0, "no folder IO");
            assert_eq!(w.ops.ops("canonical"), 0, "no advisory phase");
            assert!(lock(&w.store.reads).is_empty(), "no library file read");
        }
    }

    /// A mount not registered, a removed mount folder and a mount path that is a file skip every library item in it:
    /// the launch runs on the built-ins alone, with one `library-unavailable-skipped` notice naming the mount and its
    /// items, no library file read and no link probe; an attached spelling that is not a reference still blocks
    /// (ADR-0387 · ADR-0400 decision 19).
    #[test]
    fn an_absent_or_unregistered_mount_skips_its_library_items_and_launches_on_the_built_ins() {
        let removed = World::new();
        fs::remove_dir_all(removed.at("lib")).unwrap();
        let file = World::new();
        fs::remove_dir_all(file.at("lib")).unwrap();
        file.put("lib", "not a folder");
        let unset = World::with(Setup {
            root: false,
            ..Setup::default()
        });
        for (w, message) in [
            (&removed, "missing"),
            (&file, "not a folder"),
            (&unset, crate::resolve::NOT_REGISTERED),
        ] {
            let profile = with_default(agent(
                codex(JSON),
                &["library/P.yaml", "library/Q.yaml"],
                &w.at("work"),
            ));
            let plan = w.svc.check(&profile).unwrap();
            assert!(plan.include_priming, "{message}");
            assert!(plan.instructions.is_empty() && plan.skills.is_empty());
            assert_eq!(
                plan.notices,
                [notice(
                    LIBRARY_UNAVAILABLE_SKIPPED,
                    json!({
                        "mount": "library",
                        "message": message,
                        "items": ["library/P.yaml", "library/Q.yaml"],
                    })
                )],
                "{message}"
            );
            assert!(lock(&w.store.reads).is_empty(), "no library file read");
            assert_eq!(
                w.store.first_links.load(Ordering::SeqCst),
                0,
                "no link probe"
            );
            assert!(w.ops.ops("canonical") > 0, "the advisory phase ran");

            let unparsed = with_default(agent(
                codex(JSON),
                &["library/P.yaml", "../out.yaml"],
                &w.at("work"),
            ));
            let blocked = w.svc.check(&unparsed).unwrap_err();
            assert_eq!(
                codes(&blocked),
                [("broken", "item-path-invalid")],
                "{message}"
            );
        }
    }

    // ── the launch note (ADR-0387) ──────────────────────────────────────────────────────────────────────────────

    /// The merged instructions file `prepare` writes for `profile` (checked first), or `None`.
    fn written(w: &World, profile: &AgentProfile) -> Option<String> {
        let plan = w.svc.check(profile).unwrap();
        let kit = w.svc.prepare(plan, profile.id, 1).unwrap();
        kit.instructions_file
            .map(|file| fs::read_to_string(file).unwrap())
    }

    /// claude appends: with no instruction text of Engram's own, the note alone is written (and carried) — the named
    /// form, one line per skipped item, the folder as a folder, an attached preset as attached.
    #[test]
    fn a_claude_note_names_every_skipped_item_and_a_note_alone_is_written() {
        let w = World::new();
        let gone = w.at("gone");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[&gone]),
        );
        let profile = agent(
            claude(JSON),
            &["library/P.yaml", "library/absent.yaml"],
            &w.at("work"),
        );
        assert_eq!(
            written(&w, &profile).as_deref(),
            Some(
                format!(
                    "Engram launch note: these items your presets reference were not found and were left out of \
                     this launch:\n- library/rules/gone.md (listed in library/P.yaml)\n- library/absent.yaml (attached to this agent)\n- \
                     folder {} (not found or not a folder; listed in library/P.yaml)\n",
                    gone.display()
                )
                .as_str()
            )
        );
    }

    /// With its mount not registered the note names the mount, why it could not be used and the library items there.
    #[test]
    fn an_absent_mount_note_names_the_mount_and_its_items_in_its_sentence() {
        let w = World::with(Setup {
            root: false,
            ..Setup::default()
        });
        let profile = with_default(agent(
            claude(JSON),
            &["library/P.yaml", "library/Q.yaml"],
            &w.at("work"),
        ));
        assert_eq!(
            written(&w, &profile).as_deref(),
            Some(
                "PRIMING\n\nEngram launch note: The library mount library could not be used (not registered), so \
                 these library items were left out: library/P.yaml, library/Q.yaml\n"
            )
        );
    }

    /// codex terminal: the names go on its line when they can (named form); a name its line cannot carry gives the
    /// count form — singular and plural.
    #[test]
    fn a_codex_terminal_note_is_named_when_the_line_carries_the_names_else_counted() {
        let w = World::new();
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[]),
        );
        w.put(
            "lib/Q.yaml",
            &preset(&["library/rules/it's.md"], &[], &[], &[]),
        );
        w.put(
            "lib/R.yaml",
            &preset(
                &["library/rules/it's.md", "library/rules/gone.md"],
                &[],
                &[],
                &[],
            ),
        );
        let cwd = w.at("work");
        let named = with_default(agent(codex(TERMINAL), &["library/P.yaml"], &cwd));
        assert_eq!(
            written(&w, &named).as_deref(),
            Some(
                "PRIMING\n\nEngram launch note: these items your presets reference were not found and were left \
                 out of this launch:\n- library/rules/gone.md (listed in library/P.yaml)\n"
            )
        );
        for (attached, count) in [("library/Q.yaml", 1), ("library/R.yaml", 2)] {
            let counted = with_default(agent(codex(TERMINAL), &[attached], &cwd));
            assert_eq!(
                written(&w, &counted),
                Some(format!("PRIMING\n\n{}\n", count_note(count))),
                "{attached}"
            );
        }
        assert_eq!(
            count_note(1),
            "Engram launch note: 1 item your presets reference was not found and was left out of this launch."
        );
        assert_eq!(
            count_note(2),
            "Engram launch note: 2 items your presets reference were not found and were left out of this launch."
        );
    }

    /// codex replaces the user's own instructions, so a codex agent with no instruction text of Engram's own gets no
    /// note — none at all, or a whitespace-only priming (blank as codex reads it); the notice still names the skip.
    #[test]
    fn codex_with_no_instruction_text_of_its_own_gets_no_note() {
        let w = World::new();
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[]),
        );
        for mode in [JSON, TERMINAL] {
            let profile = agent(codex(mode), &["library/P.yaml"], &w.at("work"));
            let plan = w.svc.check(&profile).unwrap();
            assert_eq!(
                plan.notices,
                [skipped_item(
                    "library/rules/gone.md",
                    &["library/P.yaml"],
                    Some(4)
                )]
            );
            assert_eq!(written(&w, &profile), None, "{mode:?}");
        }
        let blank = World::with(Setup {
            priming: Some("  \n   \n"),
            ..Setup::default()
        });
        blank.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[]),
        );
        for mode in [JSON, TERMINAL] {
            let profile = with_default(agent(codex(mode), &["library/P.yaml"], &blank.at("work")));
            let text = written(&blank, &profile).expect("the priming is a file");
            assert!(!text.contains("launch note"), "{mode:?}: {text:?}");
        }
    }

    /// Counts the events whose message is `message`.
    struct MessageCount(&'static str, Arc<AtomicUsize>);

    impl tracing::Subscriber for MessageCount {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Message(String);
            impl tracing::field::Visit for Message {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            let mut message = Message(String::new());
            event.record(&mut message);
            if message.0 == self.0 && *event.metadata().level() == tracing::Level::WARN {
                self.1.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// Both forms refused while the text without a note fits: the launch runs with no note, the checks (as many as a
    /// launch runs) log nothing about it, and `prepare` logs it once at `warn`; the reply notice still names the skip.
    #[test]
    fn a_dropped_note_never_blocks_and_prepare_logs_it_once() {
        let mut fake = Scripted::new("codex");
        fake.refuse_holding = Some("Engram launch note");
        let leaked: &'static Scripted = Box::leak(Box::new(fake));
        let w = World::with(Setup {
            backends: Arc::new(Registry(vec![leaked as &'static dyn SettingsCheck])),
            ..Setup::default()
        });
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[]),
        );
        let profile = with_default(agent(codex(JSON), &["library/P.yaml"], &w.at("work")));
        let dropped = Arc::new(AtomicUsize::new(0));
        let subscriber = MessageCount(
            "launch note dropped; command line cannot carry it",
            Arc::clone(&dropped),
        );
        let kit = tracing::subscriber::with_default(subscriber, || {
            let scope = w.svc.begin_launch();
            let mut plans: Vec<LaunchPlan> = (0..3)
                .map(|_| w.svc.check_in(&scope, &profile).unwrap())
                .collect();
            assert_eq!(dropped.load(Ordering::SeqCst), 0, "the checks log nothing");
            let plan = plans.pop().unwrap();
            assert_eq!(
                plan.notices,
                [skipped_item(
                    "library/rules/gone.md",
                    &["library/P.yaml"],
                    Some(4)
                )]
            );
            w.svc.prepare_in(&scope, plan, profile.id, 1).unwrap()
        });
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        let text = fs::read_to_string(kit.instructions_file.unwrap()).unwrap();
        assert_eq!(text, "PRIMING\n", "no note");

        // The text without a note refused too: the launch blocks on the command line as before, and no note is
        //   dropped.
        let mut fake = Scripted::new("codex");
        fake.refuse_holding = Some("PRIMING");
        let leaked: &'static Scripted = Box::leak(Box::new(fake));
        let w = World::with(Setup {
            backends: Arc::new(Registry(vec![leaked as &'static dyn SettingsCheck])),
            ..Setup::default()
        });
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/gone.md"], &[], &[], &[]),
        );
        let blocked = w
            .svc
            .check(&with_default(agent(
                codex(JSON),
                &["library/P.yaml"],
                &w.at("work"),
            )))
            .unwrap_err();
        assert_eq!(codes(&blocked), [("broken", "codex-line-too-long")]);
    }

    #[test]
    fn without_a_library_preset_the_root_is_not_stat_ed() {
        let w = World::new();
        let profile = with_default(agent(codex(JSON), &[], &w.at("work")));
        let plan = w.svc.check(&profile).unwrap();
        assert!(plan.include_priming);
        assert_eq!(w.ops.count("stat", &w.at("lib")), 0);
    }

    #[test]
    fn a_backend_with_no_settings_check_gets_the_noop_plan_and_kit() {
        let w = World::new();
        let shell = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let profile = agent(shell, &["library/P.yaml"], &w.at("work"));
        let plan = w.svc.check(&profile).unwrap();
        assert_eq!(plan, NoopLaunchKits.check(&profile).unwrap());
        let kit = w.svc.prepare(plan, profile.id, 1).unwrap();
        assert_eq!(kit.volatile_dir, None);
        assert!(!w.at("launch").exists());
    }

    #[test]
    fn a_built_in_that_cannot_be_read_never_blocks_and_its_notice_reaches_the_plan() {
        let w = World::with(Setup {
            priming: None,
            ..Setup::default()
        });
        let plan = w
            .svc
            .check(&with_default(agent(codex(JSON), &[], &w.at("work"))))
            .unwrap();
        assert!(!plan.include_priming);
        let told: Vec<&Notice> = plan
            .notices
            .iter()
            .filter(|notice| notice.code == BUILTIN_UNAVAILABLE)
            .collect();
        assert_eq!(told.len(), 1, "{:?}", plan.notices);
        assert_eq!(told[0].args["path"], "engram/default");
    }

    #[test]
    fn a_reference_through_a_link_inside_the_library_is_path_through_link_and_never_read() {
        let w = World::new();
        let outside = w.folder("outside", &[("a.md", "A")]);
        link_dir(&outside, &w.at("lib/linked")).unwrap();
        w.put(
            "lib/P.yaml",
            &preset(&["library/linked/a.md"], &[], &[], &[]),
        );
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(
                    (item.as_str(), reason.code.as_str()),
                    ("library/linked/a.md", PATH_THROUGH_LINK)
                );
                assert_eq!(reason.line, Some(4), "the line of the reference");
            }
            other => panic!("{other:?}"),
        }
        assert!(!w.store.read_any(&outside.join("a.md")));
        assert!(!w.store.read_any(&w.at("lib/linked/a.md")));
    }

    // ── modes and the port's answers ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_priming_only_mode_carries_no_instructions_or_skills_and_names_them_in_its_notice() {
        let w = World::new();
        w.put("lib/rules/a.md", "A");
        w.put("lib/skills/alpha/SKILL.md", &skill("alpha"));
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/a.md"], &["library/skills/alpha"], &[], &[]),
        );
        let profile = with_default(agent(codex(TERMINAL), &["library/P.yaml"], &w.at("work")));
        let plan = w.svc.check(&profile).unwrap();
        assert!(plan.instructions.is_empty() && plan.skills.is_empty());
        assert!(plan.include_priming);
        assert_eq!(
            plan.notices,
            [Reason {
                code: CODEX_TERMINAL_OMITTED.to_owned(),
                args: json!({ "instructions": ["library/rules/a.md"], "skills": ["alpha"] }),
                line: None,
            }]
        );
        let kit = w.svc.prepare(plan, profile.id, 1).unwrap();
        let file = kit.instructions_file.expect("the priming is always a file");
        assert_eq!(fs::read_to_string(file).unwrap(), "PRIMING\n");
    }

    #[test]
    fn a_priming_the_terminal_line_cannot_carry_is_broken() {
        let w = World::with(Setup {
            priming: Some("say \"hi\""),
            ..Setup::default()
        });
        let blocked = w
            .svc
            .check(&with_default(agent(codex(TERMINAL), &[], &w.at("work"))))
            .unwrap_err();
        assert_eq!(codes(&blocked), [("broken", "codex-priming-unsafe-char")]);
        assert!(
            matches!(&blocked.reasons[..], [BlockReason::Broken { item, .. }] if item == "command-line:codex"),
            "{blocked:?}"
        );
    }

    #[test]
    fn the_working_folder_rule_judges_the_canonical_folder() {
        let w = World::new();
        let target = w.at("a&b");
        fs::create_dir_all(&target).unwrap();
        let cwd = w.at("cwd-link");
        link_dir(&target, &cwd).unwrap();
        let blocked = w.svc.check(&agent(codex(TERMINAL), &[], &cwd)).unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(reason.code, "codex-working-folder-unsafe");
                assert_eq!(item, &format!("working-folder:{}", lossy(&target)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn notice_codes_come_from_the_port_of_a_backend_no_code_here_names() {
        let w = World::with(Setup {
            backends: Arc::new(Registry::real_and(vec![Scripted::new("shell")])),
            ..Setup::default()
        });
        let folder = w.folder("rules", &[("RULES.md", "r")]);
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/a.md"], &[], &[], &[&folder]),
        );
        let shell = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let plan = w
            .svc
            .check(&agent(shell, &["library/P.yaml"], &w.at("work")))
            .unwrap();
        let told: Vec<(&str, &Value)> = plan
            .notices
            .iter()
            .map(|notice| (notice.code.as_str(), &notice.args))
            .collect();
        assert_eq!(
            told,
            [
                (
                    "shell-omitted",
                    &json!({ "instructions": ["library/rules/a.md"], "skills": [] })
                ),
                (
                    "shell-folder-omitted",
                    &json!({ "folder": lossy(&folder), "items": ["RULES.md"] })
                ),
            ]
        );
        assert!(plan.instructions.is_empty());
    }

    #[test]
    fn a_claude_plan_with_a_claude_md_folder_carries_no_folder_source_and_lists_nothing() {
        let w = World::new();
        let folder = w.folder("rules", &[("CLAUDE.md", "c")]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let plan = w
            .svc
            .check(&agent(claude(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert!(plan.instructions.is_empty(), "{:?}", plan.instructions);
        assert_eq!(plan.folders, [folder.clone()]);
        assert_eq!(w.ops.count("list", &folder), 0);
    }

    #[test]
    fn listing_notices_name_a_cut_and_skipped_entries_and_do_not_block() {
        let w = World::with(Setup {
            list_cap: 2,
            ..Setup::default()
        });
        let folder = w.folder(
            "rules",
            &[("a.txt", ""), ("b.txt", ""), ("c.txt", ""), ("d.txt", "")],
        );
        lock(&w.ops.state)
            .skipped
            .insert(folder.clone(), vec!["sub".to_owned()]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            plan.notices,
            [
                notice(
                    FOLDER_LISTING_SKIPPED,
                    json!({ "folder": lossy(&folder), "paths": ["sub"] })
                ),
                notice(FOLDER_LISTING_PARTIAL, json!({ "folder": lossy(&folder) })),
            ]
        );
    }

    // ── folder entries: the required phase ──────────────────────────────────────────────────────────────────────

    #[test]
    fn an_unreachable_folder_blocks_the_launch_with_one_line() {
        let w = World::with(Setup {
            waits: waits(2_000, 300),
            ..Setup::default()
        });
        let folder = w.folder("rules", &[]);
        w.ops.hold(&folder);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let scope = w.svc.begin_launch();
        let blocked = w
            .svc
            .check_in(
                &scope,
                &agent(codex(JSON), &["library/P.yaml"], &w.at("work")),
            )
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
                assert_eq!(reason.code, "folder-unreadable");
                assert_eq!(reason.args["why"], TIMEOUT);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            left(&scope).0,
            Duration::ZERO,
            "the held stat took the whole budget"
        );
    }

    #[test]
    fn a_held_file_read_in_a_folder_entry_blocks_it_and_the_check_ends_inside_the_budget() {
        let w = World::with(Setup {
            waits: waits(2_000, 300),
            ..Setup::default()
        });
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.ops.hold(&folder.join("AGENTS.md"));
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let began = Instant::now();
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        assert!(began.elapsed() < ms(2_000 + 1_000), "{:?}", began.elapsed());
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
                assert_eq!(reason.code, "folder-unreadable");
                assert_eq!(reason.args["why"], TIMEOUT);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_read_whose_worker_failed_blocks_its_folder_like_one_that_did_not_answer() {
        let w = World::new();
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        lock(&w.ops.state).panics.insert(folder.join("AGENTS.md"));
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
                assert_eq!(reason.code, "folder-unreadable");
                assert_eq!(reason.args["why"], "worker panicked");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A file below a codex folder entry that the port cannot read (an access error, not a timeout) is named in that
    /// folder's `folder-listing-skipped` notice after the listing's own skips, and the launch goes on (ADR-0387).
    #[test]
    fn an_unreadable_file_below_a_folder_entry_is_named_and_the_launch_goes_on() {
        let w = World::new();
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        lock(&w.ops.state)
            .unreadable_files
            .insert(folder.join("AGENTS.md"));
        lock(&w.ops.state)
            .skipped
            .insert(folder.clone(), vec!["sub".to_owned()]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            plan.notices,
            [notice(
                FOLDER_LISTING_SKIPPED,
                json!({ "folder": lossy(&folder), "paths": ["sub", "AGENTS.md"] })
            )]
        );
        assert_eq!(plan.folders, [folder]);
        assert!(
            plan.instructions.is_empty(),
            "the unread file is not carried"
        );
    }

    #[test]
    fn folder_io_runs_with_the_library_lock_released() {
        let w = World::with(Setup {
            merges: Merges::Defer,
            ..Setup::default()
        });
        let held = Arc::new(AtomicBool::new(false));
        let probe = |svc: std::sync::Weak<LibraryService>, held: Arc<AtomicBool>| {
            move || {
                if let Some(svc) = svc.upgrade() {
                    if svc.gate.lib().try_write().is_err() {
                        held.store(true, Ordering::SeqCst);
                    }
                }
            }
        };
        let on_ops = probe(Arc::downgrade(&w.svc), Arc::clone(&held));
        *lock(&w.ops.probe) = Some(Box::new(move |_: &Path| on_ops()));
        let on_store = probe(Arc::downgrade(&w.svc), Arc::clone(&held));
        *lock(&w.store.probe) = Some(Box::new(move |_: &Path| on_store()));
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put(
            "lib/P.yaml",
            &preset(&["library/gone.md"], &[], &[], &[&folder]),
        );
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        // ADR-0387: the missing instruction is skipped, after its link probe.
        let plan = w.svc.check(&profile).unwrap();
        assert_eq!(
            plan.notices,
            [skipped_item(
                "library/gone.md",
                &["library/P.yaml"],
                Some(4)
            )]
        );
        assert!(
            w.store.first_links.load(Ordering::SeqCst) >= 1,
            "the link probe ran"
        );
        assert!(w.ops.ops("canonical") > 0, "the advisory phase ran");
        // With the file back, the check carries it, under the probe too.
        w.put("lib/gone.md", "back");
        let plan = w.svc.check(&profile).unwrap();
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
        assert!(w.ops.ops("stat") > 0 && w.ops.ops("list") > 0 && w.ops.ops("read") > 0);
        assert!(
            w.store.read_any(&w.at("lib/P.yaml")),
            "the overlay read ran"
        );
        assert!(
            !held.load(Ordering::SeqCst),
            "an IO op ran under the library lock"
        );
    }

    #[test]
    fn a_write_lock_held_across_the_overlay_step_charges_nothing_to_the_budget() {
        let w = World::with(Setup {
            merges: Merges::Defer,
            ..Setup::default()
        });
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        let p_yaml = w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        // The overlay's read of the preset waits for the test to hold the write lock: the check then needs the read
        // lock of step 4 next.
        let (reached, at_overlay) = mpsc::channel();
        let (go, wait_go) = mpsc::channel::<()>();
        let wait_go = Mutex::new(wait_go);
        let fired = AtomicBool::new(false);
        *lock(&w.store.probe) = Some(Box::new(move |path: &Path| {
            if path == p_yaml && !fired.swap(true, Ordering::SeqCst) {
                reached.send(()).unwrap();
                lock(&wait_go).recv().unwrap();
            }
        }));
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let scope = w.svc.begin_launch();
        let (svc, in_scope) = (Arc::clone(&w.svc), scope.clone());
        let (answered, answer) = mpsc::channel();
        thread::spawn(move || answered.send(svc.check_in(&in_scope, &profile)));
        ack(&at_overlay);
        let guard = w
            .svc
            .gate
            .lib()
            .try_write()
            .expect("the check holds no library lock here");
        go.send(()).unwrap();
        assert!(
            answer.recv_timeout(ms(2_000)).is_err(),
            "the check needs the read lock"
        );
        drop(guard);
        let plan = answer
            .recv_timeout(Duration::from_secs(20))
            .expect("the check answers once the lock is free")
            .unwrap();
        assert_eq!(plan.folders, [folder]);
        let (required, _) = left(&scope);
        assert!(
            required > ms(10_000 - 1_000),
            "the 2 s lock wait was charged: {required:?} left"
        );
    }

    #[test]
    fn a_check_completes_while_a_scan_is_held_at_the_gate() {
        let w = World::new();
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[]));
        w.store.hold_whole_scans(true);
        let svc = Arc::clone(&w.svc);
        let refresh = thread::spawn(move || svc.refresh(None));
        w.store.wait_parked();
        w.svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        w.store.hold_whole_scans(false);
        refresh.join().unwrap();
    }

    // ── the advisory phase ──────────────────────────────────────────────────────────────────────────────────────

    /// B30: a check a ② row already blocks runs no advisory IO and judges the working folder as spelled; a skipped row
    /// does not block, so with only that the advisory phase runs (ADR-0387).
    #[test]
    fn a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled() {
        let w = World::new();
        let target = w.folder("plain", &[]);
        let cwd = w.at("x&y");
        link_dir(&target, &cwd).unwrap();
        w.ops.hold(&cwd);
        w.put("lib/bad.yaml", "kind: settings\nversion: [\n");
        w.put(
            "lib/P.yaml",
            &preset(&["library/gone.md"], &[], &["library/bad.yaml"], &[]),
        );
        let blocked = w
            .svc
            .check(&agent(codex(TERMINAL), &["library/P.yaml"], &cwd))
            .unwrap_err();
        assert_eq!(
            codes(&blocked),
            [
                ("broken", "yaml-syntax"),
                ("broken", "codex-working-folder-unsafe")
            ],
            "the canonical folder would pass the rule"
        );
        assert_eq!(w.ops.ops("canonical"), 0);
        assert_eq!(w.ops.ops("list"), 0);

        let skips_only = World::new();
        skips_only.put("lib/P.yaml", &preset(&["library/gone.md"], &[], &[], &[]));
        let cwd = skips_only.at("work");
        skips_only
            .svc
            .check(&agent(codex(TERMINAL), &["library/P.yaml"], &cwd))
            .unwrap();
        assert_eq!(
            skips_only.ops.count("canonical", &cwd),
            1,
            "the advisory phase ran"
        );
    }

    #[test]
    fn an_unanswered_working_folder_launches_on_the_profiles_spelling_with_no_notice() {
        let w = World::with(Setup {
            waits: waits(10_000, 300),
            ..Setup::default()
        });
        let native = w.at("home/.agents/skills");
        w.put("home/.agents/skills/zeta/SKILL.md", &skill("zeta"));
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let cwd = w.at("work");
        w.ops.hold_op("canonical", &cwd);
        let scope = w.svc.begin_launch();
        let plan = w
            .svc
            .check_in(&scope, &agent(codex(JSON), &["library/P.yaml"], &cwd))
            .unwrap();
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
        assert_eq!(
            plan.folders,
            [folder.clone()],
            "no ① line for the healthy folder"
        );
        assert_eq!(
            w.ops.count("list", &native),
            0,
            "native roots are not looked for"
        );
        assert_eq!(w.svc.working_folder(&scope, &cwd), cwd);
        assert_eq!(
            w.ops.count("canonical", &cwd),
            1,
            "the decided folder is reused"
        );
    }

    #[test]
    fn a_working_folder_that_is_gone_launches_on_the_profiles_spelling() {
        let w = World::new();
        let native = w.at("home/.agents/skills");
        w.put("home/.agents/skills/zeta/SKILL.md", &skill("zeta"));
        let cwd = w.at("gone");
        let scope = w.svc.begin_launch();
        w.svc
            .check_in(&scope, &agent(codex(JSON), &[], &cwd))
            .unwrap();
        assert_eq!(w.svc.working_folder(&scope, &cwd), cwd);
        assert_eq!(
            w.ops.count("list", &native),
            0,
            "native roots are not looked for"
        );
    }

    #[test]
    fn working_folder_reuses_the_decided_folder_and_asks_once_for_another() {
        let w = World::new();
        let cwd = w.at("work");
        let other_target = w.folder("other", &[]);
        let other = w.at("other-link");
        link_dir(&other_target, &other).unwrap();
        let scope = w.svc.begin_launch();
        w.svc
            .check_in(&scope, &agent(codex(TERMINAL), &[], &cwd))
            .unwrap();
        assert_eq!(w.svc.working_folder(&scope, &cwd), cwd);
        assert_eq!(w.ops.count("canonical", &cwd), 1);
        assert_eq!(w.svc.working_folder(&scope, &other), other_target);
        assert_eq!(w.ops.count("canonical", &other), 1);
    }

    #[test]
    fn the_backends_own_skill_roots_are_found_from_the_canonical_working_folder() {
        let w = World::new();
        fs::create_dir_all(w.at("proj/.git")).unwrap();
        w.put("proj/.agents/skills/gamma/SKILL.md", &skill("gamma"));
        let sub = w.folder("proj/sub", &[]);
        let cwd = w.at("cwd-link");
        link_dir(&sub, &cwd).unwrap();
        w.put("lib/skills/gamma/SKILL.md", &skill("gamma"));
        w.put(
            "lib/P.yaml",
            &preset(&[], &["library/skills/gamma"], &[], &[]),
        );
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &cwd))
            .unwrap();
        let mut paths = vec![
            lossy(&w.at("lib/skills/gamma")),
            lossy(&w.at("proj/.agents/skills/gamma")),
        ];
        paths.sort();
        assert_eq!(
            plan.notices,
            [notice(
                CODEX_SAME_NAME_SKILLS,
                json!({ "name": "gamma", "paths": paths })
            )]
        );
    }

    #[test]
    fn same_name_skills_in_two_roots_give_one_notice_and_an_alias_of_a_root_is_the_same_place() {
        let w = World::new();
        let a = w.folder(
            "rules-a",
            &[(".agents/skills/omega/SKILL.md", &skill("omega"))],
        );
        let c = w.folder(
            "rules-c",
            &[(".agents/skills/omega/SKILL.md", &skill("omega"))],
        );
        let d = w.folder("rules-d", &[("AGENTS.md", "no skills")]);
        let b = w.at("rules-b");
        link_dir(&a, &b).unwrap();
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&a, &b, &c, &d]));
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        let paths = [
            lossy(&a.join(".agents").join("skills").join("omega")),
            lossy(&c.join(".agents").join("skills").join("omega")),
        ];
        assert_eq!(
            plan.notices,
            [notice(
                CODEX_SAME_NAME_SKILLS,
                json!({ "name": "omega", "paths": paths })
            )]
        );
        assert_eq!(
            [&a, &b, &c, &d].map(|root| w.ops.count("canonical", root)),
            [1, 1, 1, 0],
            "only roots with skills are canonicalized"
        );
    }

    #[test]
    fn two_spellings_of_one_skill_folder_are_one_place() {
        // Only a case-insensitive file system has two spellings of one folder.
        if path_key(Path::new("A")) != path_key(Path::new("a")) {
            return;
        }
        let w = World::new();
        w.put("lib/skills/alpha/SKILL.md", &skill("alpha"));
        w.put(
            "lib/P.yaml",
            &preset(&[], &["library/skills/ALPHA"], &[], &[]),
        );
        let mut profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        // The backend's own fixed root `<CODEX_HOME>/skills` is then the library's `skills` folder.
        profile.env = vec![("CODEX_HOME".to_owned(), lossy(&w.at("lib")))];
        let plan = w.svc.check(&profile).unwrap();
        assert_eq!(
            plan.skills[0].dir,
            w.at("lib/skills/ALPHA"),
            "the reference spelling"
        );
        assert_eq!(
            w.ops.count("list", &w.at("lib/skills")),
            1,
            "the native root was read"
        );
        assert_eq!(
            w.ops.count("canonical", &w.at("lib")),
            1,
            "the library place was compared"
        );
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
    }

    #[test]
    fn a_skill_md_directly_in_a_skill_root_is_one_place_seen_from_two_roots() {
        let w = World::new();
        let cwd = w.folder("work", &[(".agents/skills/SKILL.md", &skill("omega"))]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&cwd]));
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &cwd))
            .unwrap();
        let native = cwd.join(".agents").join("skills");
        assert_eq!(w.ops.count("list", &native), 1, "the native root was read");
        assert_eq!(w.ops.count("canonical", &native), 1, "and compared");
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
    }

    #[test]
    fn a_root_whose_canonical_path_is_unknown_is_left_out_never_compared_by_its_spelling() {
        let w = World::with(Setup {
            waits: waits(10_000, 3_000),
            ..Setup::default()
        });
        let a = w.folder(
            "rules-a",
            &[(".agents/skills/omega/SKILL.md", &skill("omega"))],
        );
        let b = w.at("rules-b");
        link_dir(&a, &b).unwrap();
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&a, &b]));
        w.ops.hold_op("canonical", &b);
        let plan = w
            .svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert_eq!(
            (w.ops.count("canonical", &a), w.ops.count("canonical", &b)),
            (1, 1)
        );
        assert!(plan.notices.is_empty(), "{:?}", plan.notices);
    }

    #[test]
    fn a_hung_native_root_waits_for_the_required_phase_and_spends_only_advisory_time() {
        let w = World::with(Setup {
            waits: waits(10_000, 3_000),
            ..Setup::default()
        });
        let native = w.at("home/.agents/skills");
        fs::create_dir_all(&native).unwrap();
        w.ops.hold(&native);
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.ops.hold_op("list", &folder);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let scope = w.svc.begin_launch();
        let (svc, in_scope) = (Arc::clone(&w.svc), scope.clone());
        let check = thread::spawn(move || svc.check_in(&in_scope, &profile));
        w.ops.wait_started("list", &folder);
        assert_eq!(
            (w.ops.ops("canonical"), w.ops.count("list", &native)),
            (0, 0),
            "nothing advisory while the required listing runs"
        );
        w.ops.release_op("list", &folder);
        let plan = check.join().unwrap().unwrap();
        assert_eq!(plan.folders, [folder.clone()]);
        let calls = lock(&w.ops.state).calls.clone();
        let last_required = calls
            .iter()
            .rposition(|(op, path)| *op == "read" && path == &folder.join("AGENTS.md"))
            .unwrap();
        let native_asked = calls
            .iter()
            .position(|(op, path)| *op == "list" && path == &native)
            .expect("the native root is asked on the advisory allowance");
        assert!(
            native_asked > last_required,
            "advisory only after the required phase"
        );
        let (required, advisory) = left(&scope);
        assert_eq!(
            advisory,
            Duration::ZERO,
            "the hung root spent the allowance"
        );
        assert!(
            required > ms(10_000 - 1_500),
            "the 3 s advisory wait was charged to the required budget: {required:?} left"
        );
    }

    // ── one launch, one scope ───────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn one_scope_asks_each_folder_once_across_two_checks_and_the_prepare() {
        let w = World::new();
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let scope = w.svc.begin_launch();
        w.svc.check_in(&scope, &profile).unwrap();
        let plan = w.svc.check_in(&scope, &profile).unwrap();
        w.svc.prepare_in(&scope, plan, profile.id, 1).unwrap();
        let agents_md = folder.join("AGENTS.md");
        assert_eq!(
            (
                w.ops.count("stat", &folder),
                w.ops.count("list", &folder),
                w.ops.count("canonical", &w.at("work")),
            ),
            (1, 1, 1)
        );
        assert_eq!(
            w.ops.count("read", &agents_md),
            2,
            "one read by the checks, one fresh by prepare"
        );
    }

    #[test]
    fn an_advisory_phase_that_spent_its_allowance_leaves_the_required_budget_whole() {
        let w = World::with(Setup {
            waits: waits(4_000, 1_500),
            ..Setup::default()
        });
        let native = w.at("home/.agents/skills");
        fs::create_dir_all(&native).unwrap();
        w.ops.hold(&native);
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let scope = w.svc.begin_launch();
        w.svc.check_in(&scope, &profile).unwrap();
        let plan = w.svc.check_in(&scope, &profile).unwrap();
        assert_eq!(
            w.ops.count("list", &native),
            1,
            "the native root was asked once"
        );
        let (required, advisory) = left(&scope);
        assert_eq!(advisory, Duration::ZERO, "the allowance is spent");
        assert!(
            required > ms(4_000 - 1_500),
            "the advisory wait was charged to the required budget: {required:?} left"
        );
        w.ops.hold(&folder.join("AGENTS.md"));
        let failed = w.svc.prepare_in(&scope, plan, profile.id, 1).unwrap_err();
        assert_eq!(
            failed,
            PrepareError::Failed(format!("{FOLDER_TIMEOUT}: {}", folder.display()))
        );
        assert_eq!(
            left(&scope).0,
            Duration::ZERO,
            "the read waited all the required time left"
        );
    }

    #[test]
    fn an_empty_scope_is_a_caller_bug_that_a_release_build_survives_on_a_fresh_launch() {
        let w = World::new();
        let profile = agent(codex(JSON), &[], &w.at("work"));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            w.svc.check_in(&LaunchScope::default(), &profile)
        }));
        if cfg!(debug_assertions) {
            assert!(outcome.is_err(), "a debug build asserts");
        } else {
            assert!(outcome.unwrap().is_ok());
        }
    }

    // ── the late merge ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_merge_job_that_cannot_start_merges_in_place_before_the_check_returns() {
        let w = World::with(Setup {
            merges: Merges::Fail,
            ..Setup::default()
        });
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[]));
        w.svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        assert!(w.has("library/P.yaml"));
    }

    #[test]
    fn a_check_answers_under_a_held_write_lock_and_its_deferred_merge_lands_once_after_release() {
        let w = World::with(Setup {
            merges: Merges::Defer,
            ..Setup::default()
        });
        let folder = w.folder("rules", &[]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        w.ops.hold_op("stat", &folder);
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let svc = Arc::clone(&w.svc);
        let (answered, answer) = mpsc::channel();
        thread::spawn(move || answered.send(svc.check(&profile)));
        // The check is in its folder phase, past the merge hand-off and holding no library lock.
        w.ops.wait_started("stat", &folder);
        let guard = w
            .svc
            .gate
            .lib()
            .try_write()
            .expect("the check holds no library lock here");
        w.ops.release_op("stat", &folder);
        answer
            .recv_timeout(Duration::from_secs(20))
            .expect("the check answers while the write lock is held")
            .expect("the check passes");
        let mut jobs = w.spawn.take_deferred();
        assert_eq!(jobs.len(), 1, "one merge job");
        let merge = thread::spawn(jobs.pop().unwrap());
        let p_yaml = key(&LibPath::parse("P.yaml").unwrap());
        assert!(
            guard.catalog.get(&p_yaml).is_none(),
            "not merged while the lock is held"
        );
        drop(guard);
        merge.join().unwrap();
        assert!(w.has("library/P.yaml"));
    }

    #[test]
    fn a_delayed_merge_after_an_outside_delete_keeps_the_file_until_the_next_scan() {
        let w = World::with(Setup {
            merges: Merges::Defer,
            ..Setup::default()
        });
        w.put("lib/rules/x.md", "X");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/x.md"], &[], &[], &[]),
        );
        w.svc
            .check(&agent(codex(JSON), &["library/P.yaml"], &w.at("work")))
            .unwrap();
        fs::remove_file(w.at("lib/rules/x.md")).unwrap();
        for job in w.spawn.take_deferred() {
            job();
        }
        assert!(w.has("library/rules/x.md"), "a check never removes (B31)");
        w.svc.refresh(None);
        assert!(!w.has("library/rules/x.md"));
    }

    // ── prepare ─────────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn prepare_is_stale_on_a_vanished_source_a_vanished_folder_file_and_changed_bytes_of_the_same_size_and_time(
    ) {
        let w = World::new();
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        let a_md = w.put("lib/rules/a.md", "AAAA");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/a.md"], &[], &[], &[&folder]),
        );
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));

        let plan = w.svc.check(&profile).unwrap();
        fs::remove_file(&a_md).unwrap();
        assert!(matches!(
            w.svc.prepare(plan, profile.id, 1),
            Err(PrepareError::Stale(_))
        ));

        fs::write(&a_md, "AAAA").unwrap();
        let plan = w.svc.check(&profile).unwrap();
        fs::remove_file(folder.join("AGENTS.md")).unwrap();
        assert!(matches!(
            w.svc.prepare(plan, profile.id, 2),
            Err(PrepareError::Stale(_))
        ));

        fs::write(folder.join("AGENTS.md"), "rules").unwrap();
        let plan = w.svc.check(&profile).unwrap();
        let modified = fs::metadata(&a_md).unwrap().modified().unwrap();
        fs::write(&a_md, "BBBB").unwrap();
        fs::File::options()
            .write(true)
            .open(&a_md)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(fs::metadata(&a_md).unwrap().modified().unwrap(), modified);
        assert!(matches!(
            w.svc.prepare(plan, profile.id, 3),
            Err(PrepareError::Stale(_))
        ));
    }

    #[test]
    fn a_changed_native_skill_after_the_check_does_not_make_prepare_stale() {
        let w = World::new();
        let native_skill = w.put("home/.agents/skills/zeta/SKILL.md", &skill("zeta"));
        w.put("lib/skills/zeta/SKILL.md", &skill("zeta"));
        w.put(
            "lib/P.yaml",
            &preset(&[], &["library/skills/zeta"], &[], &[]),
        );
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let plan = w.svc.check(&profile).unwrap();
        assert_eq!(
            plan.notices.len(),
            1,
            "the native skill was read for the notice"
        );
        fs::write(&native_skill, skill("zeta") + "changed").unwrap();
        w.svc.prepare(plan, profile.id, 1).unwrap();
    }

    #[test]
    fn prepare_takes_no_library_lock_and_writes_the_merged_file_with_a_stable_hash() {
        let w = World::new();
        w.put("lib/rules/a.md", "A");
        w.put(
            "lib/P.yaml",
            &preset(&["library/rules/a.md"], &[], &[], &[]),
        );
        let profile = with_default(agent(codex(JSON), &["library/P.yaml"], &w.at("work")));
        let plan = w.svc.check(&profile).unwrap();
        let guard = write(w.svc.gate.lib());
        let svc = Arc::clone(&w.svc);
        let (first, again) = (plan.clone(), plan);
        let id = profile.id;
        let kits = thread::spawn(move || {
            (
                svc.prepare(first, id, 1).unwrap(),
                svc.prepare(again, id, 2).unwrap(),
            )
        });
        let (kit, kit2) = kits.join().unwrap();
        drop(guard);
        let file = kit.instructions_file.clone().unwrap();
        let written = fs::read(&file).unwrap();
        assert_eq!(
            String::from_utf8(written.clone()).unwrap(),
            merge_instructions(Some("PRIMING"), &[("library/rules/a.md", "A")])
        );
        assert_eq!(kit.instructions_hash, Some(sha256_hex(&written)));
        assert_eq!(kit.instructions_hash, kit2.instructions_hash);
        assert_eq!(kit.last_applied_instructions_hash, None);
        assert_eq!(file.parent(), kit.volatile_dir.as_deref());
        assert_eq!(kit.volatile_dir, Some(volatile_dir(&w.at("launch"), id, 1)));
    }

    #[test]
    fn the_merged_text_the_command_line_is_judged_on_is_the_file_prepare_writes() {
        let fake = Scripted::new("codex");
        let lines = Arc::clone(&fake.lines);
        let leaked: &'static Scripted = Box::leak(Box::new(fake));
        let w = World::with(Setup {
            backends: Arc::new(Registry(vec![leaked as &'static dyn SettingsCheck])),
            ..Setup::default()
        });
        let f = w.folder("f", &[("RULES.md", "F rules")]);
        let g = w.folder("g", &[("RULES.md", "G rules")]);
        w.put("lib/rules/a.md", "A");
        w.put("lib/rules/b.md", "B");
        w.put(
            "lib/P1.yaml",
            &preset(&["library/rules/a.md"], &[], &[], &[&f, &g]),
        );
        w.put(
            "lib/P2.yaml",
            &preset(&["library/rules/b.md"], &[], &[], &[]),
        );
        let profile = with_default(agent(
            codex(JSON),
            &["library/P1.yaml", "library/P2.yaml"],
            &w.at("work"),
        ));
        let plan = w.svc.check(&profile).unwrap();
        let (f_source, g_source) = (folder_item(&f), folder_item(&g));
        let sources: Vec<&str> = plan
            .instructions
            .iter()
            .map(|source| source.path.as_str())
            .collect();
        assert_eq!(
            sources,
            ["library/rules/a.md", &f_source, &g_source, "library/rules/b.md"],
            "a folder's instruction file right after its preset's own lines, folders in their order"
        );
        let kit = w.svc.prepare(plan, profile.id, 1).unwrap();
        let written = fs::read_to_string(kit.instructions_file.unwrap()).unwrap();
        let judged = lock(&lines).last().cloned().flatten();
        assert_eq!(judged.as_deref(), Some(written.as_str()));
        assert_eq!(
            written,
            merge_instructions(
                Some("PRIMING"),
                &[
                    ("library/rules/a.md", "A"),
                    (&f_source, "F rules"),
                    (&g_source, "G rules"),
                    ("library/rules/b.md", "B")
                ]
            )
        );
    }

    #[test]
    fn a_held_folder_file_read_in_prepare_fails_and_is_not_stale() {
        let w = World::with(Setup {
            waits: waits(3_000, 300),
            ..Setup::default()
        });
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["library/P.yaml"], &w.at("work"));
        let plan = w.svc.check(&profile).unwrap();
        w.ops.hold(&folder.join("AGENTS.md"));
        assert_eq!(
            w.svc.prepare(plan, profile.id, 1),
            Err(PrepareError::Failed(format!(
                "{FOLDER_TIMEOUT}: {}",
                folder.display()
            )))
        );
    }

    #[test]
    fn a_real_kit_materializes_for_each_registered_backend_and_release_takes_it_back() {
        for command in [claude(JSON), codex(JSON)] {
            let w = World::new();
            let skill_md = w.put("lib/skills/alpha/SKILL.md", &skill("alpha"));
            w.put("lib/rules/a.md", "A");
            w.put(
                "lib/P.yaml",
                &preset(&["library/rules/a.md"], &["library/skills/alpha"], &[], &[]),
            );
            let cwd = w.at("work");
            let profile = with_default(agent(command.clone(), &["library/P.yaml"], &cwd));
            let plan = w.svc.check(&profile).unwrap();
            let kit = w.svc.prepare(plan, profile.id, 1).unwrap();
            let artifacts = materialize(&command, &kit, &cwd, &StdLaunchFs)
                .unwrap_or_else(|e| panic!("{command:?}: {e}"));
            assert!(artifacts.real, "{command:?}");
            w.svc.release(profile.id, 1);
            assert!(!volatile_dir(&w.at("launch"), profile.id, 1).exists());
            assert!(skill_md.exists(), "the library skill survives the release");
        }
    }

    // ── the manager over this service: launch blocks at create (R4 · R4b) ──────────────────────────────────────

    struct NoStatus;

    impl engram_dashboard_agent::types::StatusSink for NoStatus {
        fn status_changed(
            &self,
            _id: AgentId,
            _status: engram_dashboard_agent::types::AgentStatus,
            _epoch: u32,
        ) {
        }
        fn agent_list_updated(&self, _agents: Vec<engram_dashboard_agent::types::AgentInfo>) {}
    }

    /// An agent manager whose launch kits are this world's service, over its own empty roster.
    fn manager_over(
        w: &World,
    ) -> (
        engram_dashboard_agent::manager::AgentManager,
        Arc<ProfileRegistry>,
    ) {
        use engram_dashboard_agent::launch::LaunchPorts;
        use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};

        let profiles = Arc::new(ProfileRegistry::new(Arc::new(NoProfiles)));
        let tracker = Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        ));
        let manager = engram_dashboard_agent::manager::AgentManager::new(
            Arc::new(NoStatus),
            Arc::clone(&profiles),
            tracker,
        )
        .with_launch_ports(LaunchPorts {
            kits: Arc::clone(&w.svc) as Arc<dyn LaunchKitSource>,
            fs: Arc::new(StdLaunchFs),
        });
        (manager, profiles)
    }

    /// The one unreadable-folder reason of a blocked create, as (item, code).
    fn blocked_folder(error: &engram_dashboard_agent::types::PtyError) -> (String, String) {
        match error {
            engram_dashboard_agent::types::PtyError::LaunchBlocked(blocked) => {
                match blocked.reasons.as_slice() {
                    [BlockReason::Broken { item, reason, .. }] => {
                        (item.clone(), reason.code.clone())
                    }
                    other => panic!("one broken reason expected: {other:?}"),
                }
            }
            other => panic!("a launch block expected: {other:?}"),
        }
    }

    /// R4 — a create whose library preset names a folder entry that does not answer is blocked by the pre-create
    /// check: nothing is added to the roster and nothing starts (a missing folder is skipped — ADR-0387).
    // ADR-0348
    #[test]
    fn a_create_with_an_unreadable_folder_entry_is_blocked_before_the_add() {
        use engram_dashboard_agent::manager::NoticeDelivery;

        let w = World::with(Setup {
            waits: waits(500, 300),
            ..Setup::default()
        });
        let held = w.folder("held", &[]);
        w.ops.hold_op("stat", &held);
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[&held]));
        let (manager, profiles) = manager_over(&w);
        let profile = agent(claude(JSON), &["library/P.yaml"], &w.at("work"));

        let not_started = manager
            .create_and_activate(&profile, NoticeDelivery::Withheld)
            .expect_err("blocked");

        assert_eq!(
            blocked_folder(&not_started.error),
            (
                folder_item(&held),
                crate::resolve::FOLDER_UNREADABLE.to_owned()
            )
        );
        assert!(!not_started.created, "nothing was added");
        assert!(profiles.get(profile.id).is_none());
        assert!(manager.list_agents().is_empty());
    }

    /// R4b — the check after the add asks a folder key the pre-create check did not: the first answer about folder `a`
    /// rewrites the preset to add a folder `b` that does not answer (the overlay re-reads presets on every check), so
    /// the prelaunch blocks after the add. The agent stays registered and asleep; `a` was asked once in the whole
    /// launch (one launch scope keeps its answer).
    // ADR-0348
    // ADR-0360
    #[test]
    fn a_folder_entry_added_between_the_checks_blocks_after_the_add() {
        use engram_dashboard_agent::manager::NoticeDelivery;

        let w = World::with(Setup {
            waits: waits(500, 300),
            ..Setup::default()
        });
        let a = w.folder("a", &[]);
        let b = w.folder("b", &[]);
        w.ops.hold_op("stat", &b);
        let preset_file = w.put("lib/P.yaml", &preset(&[], &[], &[], &[&a]));
        let rewritten = preset(&[], &[], &[], &[&a, &b]);
        let done = AtomicBool::new(false);
        let a_probe = a.clone();
        *lock(&w.ops.probe) = Some(Box::new(move |path: &Path| {
            if path == a_probe && !done.swap(true, Ordering::SeqCst) {
                fs::write(&preset_file, &rewritten).unwrap();
            }
        }));
        let (manager, profiles) = manager_over(&w);
        let profile = agent(claude(JSON), &["library/P.yaml"], &w.at("work"));

        let not_started = manager
            .create_and_activate(&profile, NoticeDelivery::Withheld)
            .expect_err("blocked after the add");

        assert_eq!(
            blocked_folder(&not_started.error),
            (
                folder_item(&b),
                crate::resolve::FOLDER_UNREADABLE.to_owned()
            )
        );
        assert!(not_started.created, "created asleep");
        assert!(profiles.get(profile.id).is_some());
        assert!(manager.list_agents().is_empty(), "nothing started");
        assert_eq!(w.ops.count("stat", &a), 1, "one launch, one answer per key");
        assert_eq!(w.ops.count("stat", &b), 1);
    }
}
