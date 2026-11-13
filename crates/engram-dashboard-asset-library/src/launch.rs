//! Launch materials — the library service as the agent manager's `LaunchKitSource`: the launch check, the materials
//! a launch gets, and their removal.
//!
//! ## Rules (invariants)
//!
//! - **One launch, one scope**: every check, the prepare and the working-folder call of one launch share one
//!   [`LaunchScope`] — one IO context (one required budget, one advisory allowance, one answer set) and the working
//!   folder the last check decided.
//! - **The check** (TRD S25 step-7 §3-8 `check_in`): the library root is stat'ed only when a library preset is
//!   attached; the library files the profile reaches are read fresh with the store (the overlay — local by
//!   assumption); two brief read-lock takes — the root and the gate's numbers before the overlay read, then `resolve`
//!   over the overlay, then the catalog (ADR-0324 decision 5). Block ④ is the whole answer — nothing
//!   else is checked. No folder IO runs under the library lock. The required phase (folder-entry stats, listings and
//!   reads) runs first and alone; the advisory phase (the working-folder canonicalize, the backend's own skill
//!   roots, root canonicals) runs only when nothing blocks, on its own allowance; an advisory answer that is missing
//!   is never substituted (an unknown root is left out, an unknown working folder is the profile's own spelling).
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
// TRD S25 step-7 §3-8 · §4 · §6

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use engram_dashboard_agent::backend::settings_check::{
    CommandLine, FolderLoadKind, FolderLoadSet, NativePlaces,
};
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::{
    backend_word, escape_controls, BlockReason, LaunchBlocked, LaunchFs, LaunchKit,
    LaunchKitSource, LaunchPlan, LaunchScope, NoopLaunchKits, Notice, PrepareError, RawSetting,
    Reason, SkillRef, SourceRef, StdLaunchFs, FOLDER_LISTING_PARTIAL, FOLDER_LISTING_SKIPPED,
};
use engram_dashboard_agent::profile::{AgentOutputFormat, AgentProfile};
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::{path_key, unlink_dir, PathKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::catalog::{key, preset_refs, Item, ItemContent, ScanItem};
use crate::folder::{slash_path, FolderListing, FolderStatus, LaunchIo, Need};
use crate::format::YamlFrontMatter;
use crate::gate::Tags;
use crate::path::{AssetRef, BuiltinId, LibPath};
use crate::resolve::{
    folder_item, resolve, CatalogLookup, ItemView, Looked, Lookup, Resolution, RootState, Target,
};
use crate::store::admits;
use crate::{extra_args, output_format, LibraryService, Want, LIBRARY_READ_CAP};

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

/// The lowercase hex SHA-256 of `bytes` — the stamp of a source and the instructions hash.
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
    /// The root the files were read under.
    root: PathBuf,
    seen: HashMap<PathKey, Seen>,
    /// Every item the reads made — the late merge's positive observations.
    items: Vec<Item>,
    /// The bytes behind each item read, by the path key of the item's disk path (a skill: its folder).
    files: HashMap<PathKey, FileRead>,
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
                    file: Some(item.path.to_path(&self.overlay.root)),
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
struct FolderReads<'a> {
    svc: &'a LibraryService,
    io: &'a LaunchIo,
    root: &'a Path,
    need: Need,
    unanswered: RefCell<Option<String>>,
    texts: RefCell<HashMap<PathBuf, String>>,
}

impl<'a> FolderReads<'a> {
    fn new(svc: &'a LibraryService, io: &'a LaunchIo, root: &'a Path, need: Need) -> Self {
        FolderReads {
            svc,
            io,
            root,
            need,
            unanswered: RefCell::new(None),
            texts: RefCell::new(HashMap::new()),
        }
    }

    fn read(&self, rel: &Path) -> io::Result<String> {
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

/// What `prepare` re-reads and compares (`LaunchPlan::stamp`, opaque to everyone else).
#[derive(Serialize, Deserialize)]
struct Stamp {
    files: Vec<StampEntry>,
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
        let (instructions_file, instructions_hash) = if priming.is_some() || !sources.is_empty() {
            let merged = merge_instructions(priming, &sources);
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
            tracing::warn!("모르는 LaunchScope가 넘어옴, 새 LaunchIo로 진행");
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

        // ADR-0324 decision 5: the numbers are noted in the read lock that reads the root the overlay reads under.
        let (noted, root) = self
            .gate
            .with_read(|state| (self.gate.numbers(), state.root.clone()));
        let library_attached = want
            .presets
            .iter()
            .any(|preset| matches!(preset, AssetRef::Library(_)));
        let root_state = self.root_state(io, root.as_deref(), Ok(()), library_attached);
        let overlay = match (&root, &root_state) {
            (Some(root), RootState::Ok) => self.read_overlay(root, &want),
            _ => Overlay::default(),
        };

        let target = Target {
            backend: want.backend,
            mode: want.mode,
            default_preset: want.default_preset,
            presets: want.presets,
            unparsed: want.unparsed,
            root: root_state,
        };
        let mut resolution = self.gate.with_read(|state| {
            let lookup = OverlayLookup {
                overlay: &overlay,
                catalog: CatalogLookup {
                    catalog: &state.catalog,
                    root: state.root.as_deref(),
                    builtins: &self.builtins,
                },
            };
            resolve(&target, &lookup, backends)
        });
        // ADR-0361
        // User decision 2026-11-12: block ④ is the whole answer — no folder IO, no command-line judgment.
        if resolution
            .problems
            .iter()
            .any(|problem| matches!(problem, BlockReason::LibraryUnavailable { .. }))
        {
            return Err(LaunchBlocked {
                reasons: resolution.problems,
            });
        }
        if let Some(root) = &root {
            self.mark_links(root, &mut resolution);
        }
        // ADR-0366
        // B26: the merge needs nothing the later steps find, so it is handed off as soon as the overlay is complete.
        if !overlay.items.is_empty() {
            self.hand_off_merge(noted, overlay.items.clone());
        }

        let (listed, mut notices) = self.folder_phase(io, check, mode, &mut resolution);

        // B30: a launch already blocked runs no advisory IO; its working folder is the profile's own spelling.
        let canonical = if resolution.problems.is_empty() {
            self.canonical_cwd(io, &profile.cwd)
        } else {
            None
        };
        if let (Some(code), Some(cwd)) = (codes.same_name, &canonical) {
            let names = self.same_names(
                io,
                check,
                mode,
                profile,
                cwd,
                root.as_deref(),
                &resolution,
                &listed,
            );
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
            .filter_map(|item| overlay.read_of(&item.path.to_path(&overlay.root)))
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
        let line = CommandLine {
            mode,
            cwd: &cwd,
            extra_args: &extra_args(&profile.command),
            config_args: &config_args,
            instructions: (!merged.is_empty()).then_some(merged.as_str()),
        };
        // ADR-0361
        if let Err(reason) = check.command_line_fits(&line) {
            resolution.problems.push(BlockReason::Broken {
                item: format!("command-line:{word}"),
                via: Vec::new(),
                reason,
            });
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
        notices.splice(0..0, resolution.notices);
        let stamp =
            serde_json::to_string(&Stamp { files: stamp }).expect("a stamp of strings serializes");
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
                .map(|folder| folder.path)
                .collect(),
            extra_skill_roots,
            stamp: Some(stamp),
        })
    }

    /// Step 3's library half: every library preset `want` reaches and every library file those presets list, read
    /// fresh with the store through the partial scan a refresh of that path runs — so a path is admitted exactly as
    /// the scan admits it (names, links, kinds) and each read draws its number first.
    // ADR-0365
    fn read_overlay(&self, root: &Path, want: &Want) -> Overlay {
        let mut overlay = Overlay {
            root: root.to_path_buf(),
            ..Overlay::default()
        };
        let mut todo: Vec<LibPath> = want
            .presets
            .iter()
            .filter_map(|preset| match preset {
                AssetRef::Library(path) => Some(path.clone()),
                AssetRef::BuiltIn(_) => None,
            })
            .collect();
        if want.default_preset {
            if let ItemContent::Preset { doc } =
                self.builtins.view(BuiltinId::DefaultPreset).content
            {
                todo.extend(preset_refs(doc).cloned());
            }
        }
        while let Some(path) = todo.pop() {
            if overlay.seen.contains_key(&key(&path)) {
                continue;
            }
            let seen = self.overlay_read(&path, &mut overlay);
            if let Seen::Item(i) = &seen {
                if let ItemContent::Preset { doc } = &overlay.items[*i].content {
                    todo.extend(preset_refs(doc).cloned());
                }
            }
            overlay.seen.insert(key(&path), seen);
        }
        overlay
    }

    /// One library path read for the overlay; the items it makes and their bytes go into `overlay`.
    fn overlay_read(&self, path: &LibPath, overlay: &mut Overlay) -> Seen {
        // The scan never admits such a name, so the catalog never holds it either.
        if !admits(path) {
            return Seen::Absent;
        }
        let bytes = RefCell::new(HashMap::new());
        let read = |file: &Path| {
            let read = self.store.read(file, LIBRARY_READ_CAP)?;
            bytes.borrow_mut().insert(path_key(file), read.clone());
            Ok(read)
        };
        let result = match self.build_scan_reading(&overlay.root, Some(path), &Tags::new(), &read) {
            Ok(result) => result,
            Err(e) => return Seen::Failed(e.to_string()),
        };
        let bytes = bytes.into_inner();
        let mut found = None;
        for entry in result.entries {
            let ScanItem::Fresh(item) = entry else {
                continue;
            };
            let disk = item.path.to_path(&overlay.root);
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
    /// listings and the port's reads of their files. A folder that is not `Ok`, whose listing fails, or one of whose
    /// reads gets no answer is a ① line. Answers the folders listed and their listing notices.
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
            let listing = listings
                .remove(&folder_key)
                .expect("list_all answers every path it is asked");
            let listing = match listing {
                Ok(listing) => listing,
                Err(status) => {
                    failed.insert(folder_key, status);
                    continue;
                }
            };
            notices.extend(listing_notices(&folder, &listing));
            let reads = FolderReads::new(self, io, &folder, Need::Required);
            let set = check.folder_load_set(
                &listing.entries,
                &|rel| reads.read(rel),
                &YamlFrontMatter,
                mode,
            );
            match reads.unanswered.into_inner() {
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
                    "작업 폴더 canonicalize 응답 없음 또는 실패, 프로필 경로로 진행: {other:?}"
                );
                None
            }
        }
    }

    /// Step 6 (b) and the same-name comparison of step 7: each skill name found in more than one place, with the
    /// places (canonical skill folders, sorted). The places = the library skills the plan carries, the skills of each
    /// listed folder entry and of each of the backend's own skill roots found from the canonical `cwd` — each root
    /// (the library root too) by its canonical path, asked on the advisory allowance, and each place counted once by
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
        root: Option<&Path>,
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
        if let Some(root) = root {
            let names = resolution
                .skills
                .iter()
                .filter_map(|(name, dir, _)| {
                    let rel = dir.strip_prefix(root).ok()?;
                    Some((name.clone(), rel.to_path_buf()))
                })
                .collect();
            roots.push((root.to_path_buf(), names));
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
                    "스킬 루트 canonicalize 실패, 같은 이름 스킬 검사에서 제외: {other:?}"
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
            tracing::warn!("home 경로 없음, native_skill_roots 조회 생략");
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
                        "native 스킬 루트 목록 읽기 실패, 같은 이름 스킬 검사에서 제외: {}",
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
                    "native 스킬 루트의 파일 읽기 TimedOut, 같은 이름 스킬 검사에서 제외: {}",
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

/// The notices of one folder entry's listing: entries left out, and a listing cut at its cap. Neither blocks.
// ADR-0363
fn listing_notices(folder: &Path, listing: &FolderListing) -> Vec<Notice> {
    let folder = folder.to_string_lossy();
    let mut notices = Vec::new();
    if !listing.skipped.is_empty() {
        let args = json!({ "folder": folder, "paths": listing.skipped });
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
/// `launch_root` is an empty report. The daemon is to call it at boot, before the service serves any launch (step 8).
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
            "삭제 실패한 항목을 남김: {}",
            escape_controls(error)
        );
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_platform::fs::link_dir;

    use super::*;

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
    }

    type Probe = Box<dyn Fn() + Send + Sync>;

    /// The real folder ops on the temp folders; a test can hold any path's ops (until it releases them or the world
    /// ends), make a path's op panic, add skipped entries to a listing, and count the ops.
    #[derive(Default)]
    struct Gated {
        state: Mutex<GatedState>,
        moved: Condvar,
        /// Runs as every op starts.
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
                probe();
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

        fn read(&self, p: &Path, cap: u64) -> io::Result<String> {
            self.enter("read", p);
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
            under: Option<&LibPath>,
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

        fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>> {
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
            for folder in ["lib", "work", "home"] {
                fs::create_dir_all(base.join(folder)).unwrap();
            }
            if let Some(priming) = setup.priming {
                put(&base, "builtin/instructions/priming.md", priming);
                put(
                    &base,
                    "builtin/presets/default.yaml",
                    "kind: preset\nversion: 1\ninstructions:\n  - engram:/priming\n",
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
                root: setup.root.then(|| base.join("lib")),
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
                &["rules/a.md"],
                &["skills/alpha"],
                &["settings/s.yaml"],
                &[&folder],
            ),
        );
        let profile = with_default(agent(codex(JSON), &["P.yaml"], &w.at("work")));

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
                ("rules/a.md", a_md.as_path()),
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
        assert_eq!(plan.raw_settings[0].preset, "P.yaml");
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
        assert!(w.has("P.yaml") && w.has("rules/a.md") && w.has("skills/alpha"));
    }

    #[test]
    fn a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order() {
        let w = World::new();
        w.put("lib/skills/a/SKILL.md", &skill("dup"));
        w.put("lib/skills/b/SKILL.md", &skill("dup"));
        w.put("lib/bad.yaml", "kind: settings\nversion: [\n");
        w.put(
            "lib/P.yaml",
            &preset(&["gone.md"], &["skills/a", "skills/b"], &["bad.yaml"], &[]),
        );
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        assert_eq!(
            codes(&blocked),
            [
                ("missing", "not-found"),
                ("broken", "yaml-syntax"),
                ("conflict", "skill-name-conflict")
            ]
        );
    }

    #[test]
    fn a_codex_json_skill_with_an_empty_description_blocks_the_check() {
        let w = World::new();
        w.put(
            "lib/skills/e/SKILL.md",
            "---\nname: e\ndescription: ''\n---\nbody\n",
        );
        w.put("lib/P.yaml", &preset(&[], &["skills/e"], &[], &[]));
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        assert!(
            matches!(&blocked.reasons[..], [BlockReason::Broken { item, .. }] if item == "skills/e"),
            "{blocked:?}"
        );
    }

    #[test]
    fn block_four_is_the_whole_answer_and_asks_nothing_else() {
        // A codex terminal agent whose working folder the terminal line cannot carry would add a ② row.
        let w = World::new();
        let unsafe_cwd = w.at("a&b");
        fs::create_dir_all(&unsafe_cwd).unwrap();
        w.put("lib/P.yaml", &preset(&[], &[], &[], &[]));
        fs::remove_dir_all(w.at("lib")).unwrap();
        let profile = with_default(agent(codex(TERMINAL), &["P.yaml"], &unsafe_cwd));
        let blocked = w.svc.check(&profile).unwrap_err();
        assert_eq!(codes(&blocked), [("library", "library-unavailable")]);
        assert_eq!(w.ops.count("stat", &w.at("lib")), 1);
        assert_eq!(w.ops.ops("canonical"), 0, "no advisory phase");
        assert!(lock(&w.store.reads).is_empty(), "no library file read");

        let unset = World::with(Setup {
            root: false,
            ..Setup::default()
        });
        let blocked = unset
            .svc
            .check(&agent(codex(TERMINAL), &["P.yaml"], &unsafe_cwd))
            .unwrap_err();
        assert_eq!(codes(&blocked), [("library", "library-unavailable")]);
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
        let profile = agent(shell, &["P.yaml"], &w.at("work"));
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
        assert_eq!(told[0].args["path"], "engram:/default");
    }

    #[test]
    fn a_reference_through_a_link_inside_the_library_is_path_through_link_and_never_read() {
        let w = World::new();
        let outside = w.folder("outside", &[("a.md", "A")]);
        link_dir(&outside, &w.at("lib/linked")).unwrap();
        w.put("lib/P.yaml", &preset(&["linked/a.md"], &[], &[], &[]));
        let blocked = w
            .svc
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Broken { item, reason, .. }] => {
                assert_eq!(
                    (item.as_str(), reason.code.as_str()),
                    ("linked/a.md", PATH_THROUGH_LINK)
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
            &preset(&["rules/a.md"], &["skills/alpha"], &[], &[]),
        );
        let profile = with_default(agent(codex(TERMINAL), &["P.yaml"], &w.at("work")));
        let plan = w.svc.check(&profile).unwrap();
        assert!(plan.instructions.is_empty() && plan.skills.is_empty());
        assert!(plan.include_priming);
        assert_eq!(
            plan.notices,
            [Reason {
                code: CODEX_TERMINAL_OMITTED.to_owned(),
                args: json!({ "instructions": ["rules/a.md"], "skills": ["alpha"] }),
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
        w.put("lib/P.yaml", &preset(&["rules/a.md"], &[], &[], &[&folder]));
        let shell = AgentCommand::Shell {
            program: "sh".to_owned(),
            args: Vec::new(),
        };
        let plan = w
            .svc
            .check(&agent(shell, &["P.yaml"], &w.at("work")))
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
                    &json!({ "instructions": ["rules/a.md"], "skills": [] })
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
            .check(&agent(claude(JSON), &["P.yaml"], &w.at("work")))
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
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
            .check_in(&scope, &agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Missing { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
                assert_eq!(reason.code, "folder-not-found");
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        assert!(began.elapsed() < ms(2_000 + 1_000), "{:?}", began.elapsed());
        match &blocked.reasons[..] {
            [BlockReason::Missing { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap_err();
        match &blocked.reasons[..] {
            [BlockReason::Missing { item, reason, .. }] => {
                assert_eq!(item, &folder_item(&folder));
                assert_eq!(reason.args["why"], "worker panicked");
            }
            other => panic!("{other:?}"),
        }
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
        *lock(&w.ops.probe) = Some(Box::new(on_ops));
        let on_store = probe(Arc::downgrade(&w.svc), Arc::clone(&held));
        *lock(&w.store.probe) = Some(Box::new(move |_: &Path| on_store()));
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        w.put("lib/P.yaml", &preset(&["gone.md"], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
        let blocked = w.svc.check(&profile).unwrap_err();
        assert_eq!(codes(&blocked), [("missing", "not-found")]);
        assert!(
            w.store.first_links.load(Ordering::SeqCst) >= 1,
            "the link probe ran"
        );
        // A passing check runs the advisory phase under the probe too.
        w.put("lib/gone.md", "back");
        w.svc.check(&profile).unwrap();
        assert!(w.ops.ops("canonical") > 0, "the advisory phase ran");
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap();
        w.store.hold_whole_scans(false);
        refresh.join().unwrap();
    }

    // ── the advisory phase ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled() {
        let w = World::new();
        let target = w.folder("plain", &[]);
        let cwd = w.at("x&y");
        link_dir(&target, &cwd).unwrap();
        w.ops.hold(&cwd);
        w.put("lib/P.yaml", &preset(&["gone.md"], &[], &[], &[]));
        let blocked = w
            .svc
            .check(&agent(codex(TERMINAL), &["P.yaml"], &cwd))
            .unwrap_err();
        assert_eq!(
            codes(&blocked),
            [
                ("missing", "not-found"),
                ("broken", "codex-working-folder-unsafe")
            ],
            "the canonical folder would pass the rule"
        );
        assert_eq!(w.ops.ops("canonical"), 0);
        assert_eq!(w.ops.ops("list"), 0);
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
            .check_in(&scope, &agent(codex(JSON), &["P.yaml"], &cwd))
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
        w.put("lib/P.yaml", &preset(&[], &["skills/gamma"], &[], &[]));
        let plan = w.svc.check(&agent(codex(JSON), &["P.yaml"], &cwd)).unwrap();
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
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
        w.put("lib/P.yaml", &preset(&[], &["skills/ALPHA"], &[], &[]));
        let mut profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
        let plan = w.svc.check(&agent(codex(JSON), &["P.yaml"], &cwd)).unwrap();
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap();
        assert!(w.has("P.yaml"));
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
        assert!(w.has("P.yaml"));
    }

    #[test]
    fn a_delayed_merge_after_an_outside_delete_keeps_the_file_until_the_next_scan() {
        let w = World::with(Setup {
            merges: Merges::Defer,
            ..Setup::default()
        });
        w.put("lib/rules/x.md", "X");
        w.put("lib/P.yaml", &preset(&["rules/x.md"], &[], &[], &[]));
        w.svc
            .check(&agent(codex(JSON), &["P.yaml"], &w.at("work")))
            .unwrap();
        fs::remove_file(w.at("lib/rules/x.md")).unwrap();
        for job in w.spawn.take_deferred() {
            job();
        }
        assert!(w.has("rules/x.md"), "a check never removes (B31)");
        w.svc.refresh(None);
        assert!(!w.has("rules/x.md"));
    }

    // ── prepare ─────────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn prepare_is_stale_on_a_vanished_source_a_vanished_folder_file_and_changed_bytes_of_the_same_size_and_time(
    ) {
        let w = World::new();
        let folder = w.folder("rules", &[("AGENTS.md", "rules")]);
        let a_md = w.put("lib/rules/a.md", "AAAA");
        w.put("lib/P.yaml", &preset(&["rules/a.md"], &[], &[], &[&folder]));
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));

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
        w.put("lib/P.yaml", &preset(&[], &["skills/zeta"], &[], &[]));
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
        w.put("lib/P.yaml", &preset(&["rules/a.md"], &[], &[], &[]));
        let profile = with_default(agent(codex(JSON), &["P.yaml"], &w.at("work")));
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
            merge_instructions(Some("PRIMING"), &[("rules/a.md", "A")])
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
        w.put("lib/P1.yaml", &preset(&["rules/a.md"], &[], &[], &[&f, &g]));
        w.put("lib/P2.yaml", &preset(&["rules/b.md"], &[], &[], &[]));
        let profile = with_default(agent(codex(JSON), &["P1.yaml", "P2.yaml"], &w.at("work")));
        let plan = w.svc.check(&profile).unwrap();
        let (f_source, g_source) = (folder_item(&f), folder_item(&g));
        let sources: Vec<&str> = plan
            .instructions
            .iter()
            .map(|source| source.path.as_str())
            .collect();
        assert_eq!(
            sources,
            ["rules/a.md", &f_source, &g_source, "rules/b.md"],
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
                    ("rules/a.md", "A"),
                    (&f_source, "F rules"),
                    (&g_source, "G rules"),
                    ("rules/b.md", "B")
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
        let profile = agent(codex(JSON), &["P.yaml"], &w.at("work"));
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
                &preset(&["rules/a.md"], &["skills/alpha"], &[], &[]),
            );
            let cwd = w.at("work");
            let profile = with_default(agent(command.clone(), &["P.yaml"], &cwd));
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
}
