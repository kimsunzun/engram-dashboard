//! The in-memory item table of the library ([`Catalog`]) and a scan's result ([`ScanResult`]).
//!
//! ## Invariants
//!
//! - **Class A — no whole-catalog swap** (TRD S25 step-7 §6): every write goes through the per-item primitives
//!   ([`Catalog::apply_items`] · [`Catalog::apply_scan`] · [`Catalog::remove`]); per item the later `read_seq` wins,
//!   and nothing outside a scan's range is touched. [`Catalog::clear`] exists for the root change only.
//! - **Every removal is sequenced**: an item goes only when an absence observed later than its read says so — a
//!   listing that lacks it (`listed_at` of its nearest listed folder) or a range observed absent. A failed read
//!   (`kept`) removes nothing.
//! - **Keys**: an item is keyed by [`key`] — the platform path key of its library path as spelled from the root, so
//!   two spellings that differ only in case (on Windows) are one item. The root is the same for every item, so the
//!   key does not include it.
//! - **Derived data** — `refs_to` (target → the presets that list it directly, keeping the keys of missing targets)
//!   is kept in step with every item write; `used_by` (preset → every key it reaches, directly or through nested
//!   presets) is recomputed by [`Catalog::recompute_derived`] for the touched keys, walking upward through `refs_to`
//!   with a visited set — never by rebuilding the catalog. Agent-side references are not stored (read from the
//!   profile registry — `refs`).
//! - The catalog holds no IO: [`ScanResult::build`] reads through a closure, outside any lock.
// ADR-0324
// ADR-0341
// TRD S25 step-7 §3-5 · §6

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use engram_dashboard_agent::launch::Reason;
use engram_dashboard_platform::fs::{path_key, same_path, PathKey};
use serde::Deserialize;

use crate::backends::Backends;
use crate::excerpt::excerpt;
use crate::format::{
    library_skill_name, options, parse_preset, parse_settings, sniff_kind, split_front_matter,
    ParseError, PresetDoc, SettingsDoc, Sniff, YAML_SYNTAX,
};
use crate::path::{AssetRef, LibPath};
use crate::store::{ContentTag, ScanListing};
use crate::validate::{check_settings, SettingsVerdict};

/// A file whose modification time is this close to (or after) the time it was read may change again within the
/// file system's time resolution without a new tag, so it is re-read by the next scan even when its tag is equal (the
/// racy rule).
pub const RACY_WINDOW: Duration = Duration::from_secs(2);

const SKILL_FILE: &str = "SKILL.md";

/// The catalog key of a library path (module header): the platform path key of its spelling from the root. Every
/// [`Catalog`] lookup takes this key — not [`LibPath::key`], which keys the absolute disk path and matches nothing
/// here.
pub fn key(path: &LibPath) -> PathKey {
    path_key(Path::new(path.as_str()))
}

/// One library item.
#[derive(Clone, Debug)]
pub struct Item {
    /// A skill's path is its folder; every other item's is its file.
    pub path: LibPath,
    pub kind: ItemKind,
    /// The tag of the file read (a skill: its `SKILL.md`).
    pub tag: ContentTag,
    /// The sequence number noted before the read this item was made from.
    pub read_seq: u64,
    pub content: ItemContent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Instruction,
    /// A folder holding a `SKILL.md` directly.
    Skill,
    /// The `SKILL.md` of a skill folder — stands in for the skill, is not an instruction.
    SkillFile,
    Preset,
    Settings,
    /// A `.yaml` / `.yml` that claims a kind but does not read; `claimed` = what [`sniff_kind`] found.
    Broken {
        claimed: Option<Sniff>,
    },
}

#[derive(Clone, Debug)]
pub enum ItemContent {
    Instruction {
        excerpt: String,
    },
    Skill {
        name: String,
        description: String,
        /// The `SKILL.md` name as on disk.
        file_name: String,
        /// The skill folder's name.
        folder: String,
        /// The whole `SKILL.md` text, BOM kept — `resolve` runs the port's skill format check on it.
        text: Arc<str>,
    },
    SkillFile,
    Preset {
        doc: PresetDoc,
    },
    Settings {
        doc: SettingsDoc,
        verdict: SettingsVerdict,
    },
    Broken {
        reason: Reason,
    },
}

/// A scan's outcome for one file: re-read, or seen with an unchanged tag.
#[derive(Clone, Debug)]
pub enum ScanItem {
    /// Re-read, carrying its `read_seq`.
    Fresh(Item),
    /// The tag equals the catalog's and the item is not racy — nothing is written.
    Unchanged(PathKey, ContentTag),
}

/// A scan's result, built outside the library lock (walk + read + parse). It holds no catalog (class A).
#[derive(Clone, Debug)]
pub struct ScanResult {
    /// `None` = the whole library.
    pub range: Option<LibPath>,
    pub entries: Vec<ScanItem>,
    /// The folders the listing held.
    pub folders: Vec<LibPath>,
    /// From the listing — the absence numbers (`None` = the root).
    pub listed_at: HashMap<Option<LibPath>, u64>,
    /// Whole ranges observed absent (a link, or `under` not found), each with the number drawn before the observation.
    pub absent_ranges: Vec<(LibPath, u64)>,
    /// Folders (`None` = the root) and files whose read failed — their items are left as they are.
    pub kept: Vec<Option<LibPath>>,
    pub skipped_links: Vec<LibPath>,
    pub errors: Vec<(Option<LibPath>, String)>,
    /// `.yaml` / `.yml` files re-read that declare no `kind` — not library items (`refresh` reports them).
    pub no_kind: Vec<LibPath>,
    /// The time given to [`ScanResult::build`] — the racy rule's reference.
    pub scanned_at: SystemTime,
}

impl ScanResult {
    /// Turns a listing into items: a file is re-read only when its tag differs from the one in `tags` (the catalog's
    /// items of the range, read under a brief read lock) or `tags` marks it racy. Settings verdicts come from
    /// [`check_settings`]. `read` gets the disk path of each file to re-read; a failed read keeps the file's item.
    ///
    /// `now` is the racy rule's reference and must be a time taken no later than the scan's first read — before the
    /// walk. A later time would judge a file rewritten during the scan as settled.
    pub fn build(
        listing: ScanListing,
        range: Option<&LibPath>,
        tags: &HashMap<PathKey, (ContentTag, bool)>,
        read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
        backends: &dyn Backends,
        now: SystemTime,
    ) -> ScanResult {
        let ScanListing {
            root,
            files,
            folders,
            listed_at,
            absent_ranges,
            skipped_links,
            mut errors,
        } = listing;
        let mut kept: Vec<Option<LibPath>> = errors.iter().map(|(at, _)| at.clone()).collect();
        let mut entries = Vec::new();
        let mut no_kind = Vec::new();
        for file in files {
            let Some(kind) = FileKind::of(&file.rel) else {
                continue;
            };
            let file_key = key(&file.rel);
            let skill_folder = match &kind {
                FileKind::SkillFile(folder) => Some(folder.clone()),
                _ => None,
            };
            let unchanged = |k: &PathKey| {
                tags.get(k)
                    .is_some_and(|(tag, racy)| *tag == file.tag && !racy)
            };
            let folder_unchanged = skill_folder.as_ref().is_none_or(|f| unchanged(&key(f)));
            if unchanged(&file_key) && folder_unchanged {
                entries.push(ScanItem::Unchanged(file_key, file.tag));
                if let Some(folder) = &skill_folder {
                    entries.push(ScanItem::Unchanged(key(folder), file.tag));
                }
                continue;
            }
            let bytes = match read(&file.rel.to_path(&root)) {
                Ok(bytes) => bytes,
                Err(e) => {
                    kept.push(Some(file.rel.clone()));
                    errors.push((Some(file.rel), e.to_string()));
                    continue;
                }
            };
            let item = |path: LibPath, kind: ItemKind, content: ItemContent| Item {
                path,
                kind,
                tag: file.tag,
                read_seq: file.seq,
                content,
            };
            // Instruction and skill text has no encoding rule and no reason code for bytes that are not UTF-8 (TRD
            //   S25 step-7 B8): such a byte becomes U+FFFD here.
            match kind {
                FileKind::Instruction => {
                    let text = String::from_utf8_lossy(&bytes);
                    let content = ItemContent::Instruction {
                        excerpt: excerpt(&text),
                    };
                    entries.push(ScanItem::Fresh(item(
                        file.rel,
                        ItemKind::Instruction,
                        content,
                    )));
                }
                FileKind::SkillFile(folder) => {
                    let text: Arc<str> = Arc::from(String::from_utf8_lossy(&bytes));
                    let content = ItemContent::Skill {
                        name: library_skill_name(Some(&text), folder.file_name()),
                        description: skill_description(&text),
                        file_name: file.rel.file_name().to_owned(),
                        folder: folder.file_name().to_owned(),
                        text,
                    };
                    entries.push(ScanItem::Fresh(item(folder, ItemKind::Skill, content)));
                    entries.push(ScanItem::Fresh(item(
                        file.rel,
                        ItemKind::SkillFile,
                        ItemContent::SkillFile,
                    )));
                }
                FileKind::Yaml => match yaml_item(&bytes, backends) {
                    None => no_kind.push(file.rel),
                    Some((kind, content)) => {
                        entries.push(ScanItem::Fresh(item(file.rel, kind, content)))
                    }
                },
            }
        }
        ScanResult {
            range: range.cloned(),
            entries,
            folders,
            listed_at,
            absent_ranges,
            kept,
            skipped_links,
            errors,
            no_kind,
            scanned_at: now,
        }
    }
}

/// The kind decision by name (TRD S25 §1-2 table); content decides only between the `.yaml` kinds.
enum FileKind {
    Instruction,
    /// A `SKILL.md` directly in a folder below the root; carries that folder (the skill's path).
    SkillFile(LibPath),
    Yaml,
}

impl FileKind {
    // Names compare by the OS rule (platform `same_path` — on Windows `skill.md` is a `SKILL.md`). A `SKILL.md`
    //   directly under the root has no skill folder (the root is not an item), so it is an instruction.
    fn of(rel: &LibPath) -> Option<FileKind> {
        let name = rel.file_name();
        if same_path(Path::new(name), Path::new(SKILL_FILE)) {
            if let Some(folder) = rel.parent() {
                return Some(FileKind::SkillFile(folder));
            }
        }
        let (_, extension) = name.rsplit_once('.')?;
        let is = |want: &str| same_path(Path::new(extension), Path::new(want));
        if is("md") {
            Some(FileKind::Instruction)
        } else if is("yaml") || is("yml") {
            Some(FileKind::Yaml)
        } else {
            None
        }
    }
}

/// The item a `.yaml` / `.yml` makes, or `None` when it declares no `kind` (not ours).
fn yaml_item(bytes: &[u8], backends: &dyn Backends) -> Option<(ItemKind, ItemContent)> {
    let lossy = String::from_utf8_lossy(bytes);
    let claimed = match sniff_kind(&lossy) {
        Sniff::Absent => return None,
        claimed => claimed,
    };
    let broken = |reason: Reason| {
        Some((
            ItemKind::Broken {
                claimed: Some(claimed.clone()),
            },
            ItemContent::Broken { reason },
        ))
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return broken(
            ParseError::new(YAML_SYNTAX, "the file is not valid UTF-8", None).to_reason(),
        );
    };
    match &claimed {
        Sniff::Settings => match parse_settings(text) {
            Ok(doc) => {
                let verdict = check_settings(&doc, backends);
                Some((ItemKind::Settings, ItemContent::Settings { doc, verdict }))
            }
            Err(e) => broken(e.to_reason()),
        },
        // An unknown `kind` value is read as a preset too: the reader's own error (`kind-unknown` with its line, or
        //   a syntax error) is the reason.
        Sniff::Preset | Sniff::Unknown(_) => match parse_preset(text, backends) {
            Ok(doc) => Some((ItemKind::Preset, ItemContent::Preset { doc })),
            Err(e) => broken(e.to_reason()),
        },
        Sniff::Absent => unreachable!("returned above"),
    }
}

/// The front-matter `description` of a `SKILL.md`, trimmed; empty when there is none or the front matter does not
/// parse (display only — the port's skill check judges the file).
fn skill_description(text: &str) -> String {
    #[derive(Deserialize)]
    struct DescriptionOnly {
        #[serde(default)]
        description: Option<String>,
    }
    split_front_matter(text)
        .and_then(|front| {
            serde_saphyr::from_str_with_options::<DescriptionOnly>(front.yaml, options()).ok()
        })
        .and_then(|front| front.description)
        .map(|description| description.trim().to_owned())
        .unwrap_or_default()
}

/// The item table (module header).
#[derive(Clone, Debug)]
pub struct Catalog {
    items: HashMap<PathKey, Arc<Item>>,
    folders: HashMap<PathKey, LibPath>,
    /// Preset → every key it reaches (present or missing), itself excluded.
    used_by: HashMap<PathKey, BTreeSet<PathKey>>,
    /// Target (present or missing) → the presets that list it directly.
    refs_to: HashMap<PathKey, BTreeSet<PathKey>>,
    racy: HashSet<PathKey>,
    scanned_at: SystemTime,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
            folders: HashMap::new(),
            used_by: HashMap::new(),
            refs_to: HashMap::new(),
            racy: HashSet::new(),
            scanned_at: SystemTime::UNIX_EPOCH,
        }
    }
}

impl Catalog {
    /// The item at `key` — a [`key`] (catalog key), not a [`LibPath::key`].
    pub fn get(&self, key: &PathKey) -> Option<&Item> {
        self.items.get(key).map(Arc::as_ref)
    }

    /// The items directly in `folder` (`None` = the root), by path — compared by [`key`].
    pub fn children(&self, folder: Option<&LibPath>) -> Vec<&Item> {
        let want = folder.map(key);
        let mut children: Vec<&Item> = self
            .items
            .values()
            .filter(|item| item.path.parent().as_ref().map(key) == want)
            .map(Arc::as_ref)
            .collect();
        children.sort_by(|a, b| a.path.cmp(&b.path));
        children
    }

    /// The folders the last scans listed, in no order.
    pub fn folders(&self) -> impl Iterator<Item = &LibPath> {
        self.folders.values()
    }

    /// The tag and racy mark of every item in `range` (`None` = all) — what [`ScanResult::build`] compares against.
    pub fn tags(&self, range: Option<&LibPath>) -> HashMap<PathKey, (ContentTag, bool)> {
        self.items
            .iter()
            .filter(|(_, item)| within(&item.path, range))
            .map(|(k, item)| (k.clone(), (item.tag, self.racy.contains(k))))
            .collect()
    }

    /// The time given to the last applied scan.
    pub fn scanned_at(&self) -> SystemTime {
        self.scanned_at
    }

    /// The presets that reach `key` directly or through nested presets (`key` itself excluded), by path.
    pub fn presets_reaching(&self, key: &PathKey) -> Vec<&Item> {
        let mut presets: Vec<&Item> = self
            .used_by
            .iter()
            .filter(|(preset, reached)| *preset != key && reached.contains(key))
            .filter_map(|(preset, _)| self.get(preset))
            .collect();
        presets.sort_by(|a, b| a.path.cmp(&b.path));
        presets
    }

    /// The one write primitive for observations (class A): per item, an item with a later `read_seq` than the
    /// current one replaces it; an item at a path not in the catalog is inserted only when `may_insert` (ADR-0324
    /// decision 5). Every item applied here is marked racy, so the next scan re-reads it: when the read happened is
    /// not known here, and a merge can land long after it (behind the write lock). Returns the touched keys — pass
    /// them to [`Catalog::recompute_derived`].
    pub fn apply_items(&mut self, items: Vec<Item>, may_insert: bool) -> Vec<PathKey> {
        items
            .into_iter()
            .filter_map(|item| self.apply_one(item, may_insert, None))
            .collect()
    }

    /// A scan's apply: `Fresh` entries per item as [`Catalog::apply_items`] does, except that the racy mark is judged
    /// against the scan's time instead of being set on every item; an
    /// `Unchanged` entry writes nothing. An item of the range the listing lacks is removed only when the `listed_at` of
    /// its nearest listed folder (for a skill: its own folder first) is later than its `read_seq`, and not when that
    /// folder or the file is in `kept`; an item inside an absent range is removed when the range's number is later.
    /// Items outside the range are untouched. Returns the touched keys (removed ones included).
    pub fn apply_scan(&mut self, result: ScanResult) -> Vec<PathKey> {
        let ScanResult {
            range,
            entries,
            folders,
            listed_at,
            absent_ranges,
            kept,
            scanned_at,
            ..
        } = result;
        self.scanned_at = scanned_at;
        let mut touched = Vec::new();
        let mut seen: HashSet<PathKey> = HashSet::new();
        for entry in entries {
            match entry {
                ScanItem::Fresh(item) => {
                    seen.insert(key(&item.path));
                    touched.extend(self.apply_one(item, true, Some(scanned_at)));
                }
                ScanItem::Unchanged(k, _) => {
                    seen.insert(k);
                }
            }
        }
        for folder in folders {
            self.folders.insert(key(&folder), folder);
        }
        let kept: HashSet<Option<PathKey>> = kept.iter().map(|at| at.as_ref().map(key)).collect();
        let listed: HashMap<Option<PathKey>, u64> = listed_at
            .iter()
            .map(|(at, seq)| (at.as_ref().map(key), *seq))
            .collect();
        let absent: Vec<(PathKey, u64)> = self
            .items
            .iter()
            .filter(|(k, item)| within(&item.path, range.as_ref()) && !seen.contains(*k))
            .filter(|(k, item)| {
                !kept.contains(&Some((*k).clone())) && !kept.contains(&skill_file_key(item))
            })
            .filter_map(|(k, item)| {
                let anchor = match item.kind {
                    ItemKind::Skill => Some(item.path.clone()),
                    _ => item.path.parent(),
                };
                absence_number(anchor, &listed, &kept).map(|at| (k.clone(), at))
            })
            .collect();
        for (k, at) in absent {
            if self.remove(&k, at) {
                touched.push(k);
            }
        }
        // A folder of the range the listing lacks goes unless its absence has no number (it or a folder above it
        //   failed to read).
        self.folders.retain(|k, folder| {
            !within(folder, range.as_ref())
                || listed.contains_key(&Some(k.clone()))
                || absence_number(Some(folder.clone()), &listed, &kept).is_none()
        });
        for (range, at) in absent_ranges {
            let inside: Vec<PathKey> = self
                .items
                .iter()
                .filter(|(_, item)| within(&item.path, Some(&range)))
                .map(|(k, _)| k.clone())
                .collect();
            for k in inside {
                if self.remove(&k, at) {
                    touched.push(k);
                }
            }
            self.folders
                .retain(|_, folder| !within(folder, Some(&range)));
        }
        touched
    }

    /// Removes the item at `key` only when `absent_at` is later than its `read_seq`. `true` = removed; pass a removed
    /// key to [`Catalog::recompute_derived`].
    pub fn remove(&mut self, key: &PathKey, absent_at: u64) -> bool {
        match self.items.get(key) {
            Some(item) if absent_at > item.read_seq => {
                self.replace(key, None);
                true
            }
            _ => false,
        }
    }

    /// Recomputes `used_by` for every touched preset and every preset that reaches a touched key (found upward through
    /// `refs_to`, each visited once — a preset cycle ends the walk).
    pub fn recompute_derived(&mut self, touched: &[PathKey]) {
        let mut visited: HashSet<PathKey> = HashSet::new();
        let mut queue: Vec<PathKey> = touched.to_vec();
        while let Some(k) = queue.pop() {
            if !visited.insert(k.clone()) {
                continue;
            }
            if let Some(referrers) = self.refs_to.get(&k) {
                queue.extend(referrers.iter().cloned());
            }
        }
        for k in visited {
            if self.is_preset(&k) {
                let reached = self.reach(&k);
                self.used_by.insert(k, reached);
            } else {
                self.used_by.remove(&k);
            }
        }
    }

    /// Root change only (TRD S25 step-7 §6 writer 6): empties items, folders, tags, racy marks, `used_by` and
    /// `refs_to`. Called only in the write lock that also applies the new root's fresh build.
    pub fn clear(&mut self) {
        self.items.clear();
        self.folders.clear();
        self.used_by.clear();
        self.refs_to.clear();
        self.racy.clear();
    }

    /// `read_at` = the racy rule's reference; `None` = unknown, the item is marked racy.
    fn apply_one(
        &mut self,
        item: Item,
        may_insert: bool,
        read_at: Option<SystemTime>,
    ) -> Option<PathKey> {
        let k = key(&item.path);
        match self.items.get(&k) {
            Some(current) if item.read_seq <= current.read_seq => return None,
            None if !may_insert => return None,
            _ => {}
        }
        if read_at.is_none_or(|at| is_racy(&item.tag, at)) {
            self.racy.insert(k.clone());
        } else {
            self.racy.remove(&k);
        }
        self.replace(&k, Some(Arc::new(item)));
        Some(k)
    }

    /// Puts `new` at `k` (or removes it) and moves `k`'s direct-reference edges in `refs_to` from the old item's
    /// references to the new one's.
    fn replace(&mut self, k: &PathKey, new: Option<Arc<Item>>) {
        let old = match &new {
            Some(item) => self.items.insert(k.clone(), item.clone()),
            None => {
                self.racy.remove(k);
                self.items.remove(k)
            }
        };
        for target in old.as_deref().map(direct_refs).unwrap_or_default() {
            if let Some(referrers) = self.refs_to.get_mut(&target) {
                referrers.remove(k);
                if referrers.is_empty() {
                    self.refs_to.remove(&target);
                }
            }
        }
        for target in new.as_deref().map(direct_refs).unwrap_or_default() {
            self.refs_to.entry(target).or_default().insert(k.clone());
        }
    }

    fn is_preset(&self, k: &PathKey) -> bool {
        self.get(k)
            .is_some_and(|item| item.kind == ItemKind::Preset)
    }

    /// Every key `start` reaches through its lists and the lists of the presets it reaches (present ones only),
    /// `start` excluded.
    fn reach(&self, start: &PathKey) -> BTreeSet<PathKey> {
        let mut reached = BTreeSet::new();
        let mut visited: HashSet<PathKey> = HashSet::from([start.clone()]);
        let mut queue = vec![start.clone()];
        while let Some(preset) = queue.pop() {
            let Some(item) = self.get(&preset) else {
                continue;
            };
            for target in direct_refs(item) {
                if &target != start {
                    reached.insert(target.clone());
                }
                if visited.insert(target.clone()) && self.is_preset(&target) {
                    queue.push(target);
                }
            }
        }
        reached
    }
}

/// The library references a preset item lists directly — built-ins and folder entries are not library references
/// (ADR-0341).
pub(crate) fn preset_refs(doc: &PresetDoc) -> impl Iterator<Item = &LibPath> {
    [&doc.presets, &doc.instructions, &doc.skills, &doc.settings]
        .into_iter()
        .flatten()
        .filter_map(|item| match &item.target {
            AssetRef::Library(path) => Some(path),
            AssetRef::BuiltIn(_) => None,
        })
}

fn direct_refs(item: &Item) -> BTreeSet<PathKey> {
    match &item.content {
        ItemContent::Preset { doc } => preset_refs(doc).map(key).collect(),
        _ => BTreeSet::new(),
    }
}

/// For a skill item, the key of the `SKILL.md` it was read from (a failed read of that file keeps the skill too).
fn skill_file_key(item: &Item) -> Option<PathKey> {
    let ItemContent::Skill { file_name, .. } = &item.content else {
        return None;
    };
    LibPath::parse(&format!("{}/{file_name}", item.path.as_str()))
        .ok()
        .map(|file| key(&file))
}

fn within(path: &LibPath, range: Option<&LibPath>) -> bool {
    range.is_none_or(|range| path.replace_prefix(range, range).is_some())
}

fn is_racy(tag: &ContentTag, read_at: SystemTime) -> bool {
    read_at
        .checked_sub(RACY_WINDOW)
        .is_none_or(|edge| tag.modified >= edge)
}

/// The number an absence below `anchor` was observed at: the `listed_at` of the nearest listed folder from `anchor`
/// up. `None` = no number (a folder on the way is in `kept`, or nothing above was listed).
fn absence_number(
    mut anchor: Option<LibPath>,
    listed: &HashMap<Option<PathKey>, u64>,
    kept: &HashSet<Option<PathKey>>,
) -> Option<u64> {
    loop {
        let at = anchor.as_ref().map(key);
        if kept.contains(&at) {
            return None;
        }
        if let Some(seq) = listed.get(&at) {
            return Some(*seq);
        }
        anchor = anchor?.parent();
    }
}

/// A library on a `MemStore` with a catalog — shared by the catalog and refs tests.
#[cfg(test)]
pub(crate) mod harness {
    use std::cell::Cell;
    use std::path::PathBuf;

    use super::*;
    use crate::backends::fake::{Registry, Scripted};
    use crate::store::mem::{MemStore, OLD};
    use crate::store::LibraryStore;

    /// Far from every file's [`OLD`] modification time, so nothing is racy unless a test says so.
    pub(crate) fn now() -> SystemTime {
        OLD + Duration::from_secs(1_000_000)
    }

    pub(crate) fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    /// A library on a `MemStore` with a catalog and one increasing sequence, like the service's gate.
    pub(crate) struct Lib {
        pub(crate) root: PathBuf,
        pub(crate) store: MemStore,
        pub(crate) catalog: Catalog,
        next: Cell<u64>,
        pub(crate) backends: Registry,
    }

    impl Lib {
        pub(crate) fn new() -> Self {
            let root = PathBuf::from("/lib");
            Self {
                store: MemStore::new(&root),
                root,
                catalog: Catalog::default(),
                next: Cell::new(0),
                backends: Registry::real_and(vec![Scripted::new("gamma")]),
            }
        }

        pub(crate) fn seq(&self) -> u64 {
            self.next.set(self.next.get() + 1);
            self.next.get()
        }

        /// Walk + read + parse, nothing applied.
        pub(crate) fn build(&self, under: Option<&str>) -> ScanResult {
            let under = under.map(lp);
            let listing = self
                .store
                .scan(&self.root, under.as_ref(), &|| self.seq())
                .unwrap();
            let tags = self.catalog.tags(under.as_ref());
            ScanResult::build(
                listing,
                under.as_ref(),
                &tags,
                &self.store.reader(),
                &self.backends,
                now(),
            )
        }

        pub(crate) fn apply(&mut self, result: ScanResult) -> Vec<PathKey> {
            let touched = self.catalog.apply_scan(result);
            self.catalog.recompute_derived(&touched);
            touched
        }

        pub(crate) fn scan(&mut self, under: Option<&str>) -> Vec<PathKey> {
            let result = self.build(under);
            self.apply(result)
        }

        /// What a launch check's overlay read of `rel` would merge: an instruction read now.
        pub(crate) fn check_reads(&mut self, rel: &str) -> Vec<PathKey> {
            let item = self.overlay_read(rel);
            let touched = self.catalog.apply_items(vec![item], true);
            self.catalog.recompute_derived(&touched);
            touched
        }

        /// An instruction as a launch check's overlay reads it: the number is drawn before the disk is touched.
        pub(crate) fn overlay_read(&self, rel: &str) -> Item {
            let path = lp(rel);
            let disk = path.to_path(&self.root);
            let read_seq = self.seq();
            let tag = self.store.tag(&disk).unwrap();
            let text = String::from_utf8(self.store.read(&disk, u64::MAX).unwrap()).unwrap();
            Item {
                path,
                kind: ItemKind::Instruction,
                tag,
                read_seq,
                content: ItemContent::Instruction {
                    excerpt: excerpt(&text),
                },
            }
        }

        pub(crate) fn item(&self, rel: &str) -> Option<&Item> {
            self.catalog.get(&key(&lp(rel)))
        }

        pub(crate) fn excerpt_of(&self, rel: &str) -> String {
            match &self.item(rel).expect(rel).content {
                ItemContent::Instruction { excerpt } => excerpt.clone(),
                other => panic!("{rel}: {other:?}"),
            }
        }

        pub(crate) fn reaching(&self, rel: &str) -> Vec<&str> {
            self.catalog
                .presets_reaching(&key(&lp(rel)))
                .into_iter()
                .map(|item| item.path.as_str())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::harness::{lp, now, Lib};
    use super::*;
    use crate::store::mem::ListFailure;
    use crate::store::LibraryStore;

    fn fresh_paths(result: &ScanResult) -> Vec<&str> {
        let mut paths: Vec<&str> = result
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ScanItem::Fresh(item) => Some(item.path.as_str()),
                ScanItem::Unchanged(..) => None,
            })
            .collect();
        paths.sort_unstable();
        paths
    }

    fn unchanged_count(result: &ScanResult) -> usize {
        result
            .entries
            .iter()
            .filter(|entry| matches!(entry, ScanItem::Unchanged(..)))
            .count()
    }

    // ── the kind decision ─────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn kinds_are_decided_by_name_and_for_yaml_by_the_declared_kind() {
        let mut lib = Lib::new();
        lib.store.file("rules/a.md", "# T\n\nFirst paragraph.\n");
        lib.store.file(
            "skills/review/SKILL.md",
            "---\nname:  Code   Review \ndescription: \" Reviews code \"\n---\nbody\n",
        );
        lib.store
            .file("skills/review/notes.md", "Inside a skill.\n");
        lib.store.file(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/a.md\n",
        );
        lib.store
            .file("s.yml", "kind: settings\nversion: 1\ngamma:\n  unsafe: 1\n");
        lib.store.file("bad.yaml", "kind: preset\nversion: [1\n");
        lib.store.file("typo.yaml", "kind: prest\nversion: 1\n");
        lib.store.file("plain.yaml", "name: not ours\n");
        lib.store.file("image.png", "png");
        lib.store.file("README.txt", "text");
        lib.store
            .file("SKILL.md", "A root SKILL.md has no skill folder.\n");
        let result = lib.build(None);
        assert_eq!(result.no_kind, [lp("plain.yaml")]);
        lib.apply(result);

        let kind = |rel: &str| lib.item(rel).map(|item| item.kind.clone());
        assert_eq!(kind("rules/a.md"), Some(ItemKind::Instruction));
        assert_eq!(lib.excerpt_of("rules/a.md"), "First paragraph.");
        assert_eq!(kind("skills/review"), Some(ItemKind::Skill));
        assert_eq!(kind("skills/review/SKILL.md"), Some(ItemKind::SkillFile));
        assert_eq!(kind("skills/review/notes.md"), Some(ItemKind::Instruction));
        assert_eq!(kind("SKILL.md"), Some(ItemKind::Instruction));
        assert_eq!(kind("p.yaml"), Some(ItemKind::Preset));
        assert_eq!(kind("s.yml"), Some(ItemKind::Settings));
        assert_eq!(
            kind("bad.yaml"),
            Some(ItemKind::Broken {
                claimed: Some(Sniff::Preset)
            })
        );
        assert_eq!(
            kind("typo.yaml"),
            Some(ItemKind::Broken {
                claimed: Some(Sniff::Unknown("prest".to_owned()))
            })
        );
        for absent in ["plain.yaml", "image.png", "README.txt"] {
            assert_eq!(kind(absent), None, "{absent}");
        }

        let ItemContent::Skill {
            name,
            description,
            file_name,
            folder,
            text,
        } = &lib.item("skills/review").unwrap().content
        else {
            panic!("not a skill");
        };
        assert_eq!(
            (name.as_str(), description.as_str()),
            ("Code Review", "Reviews code")
        );
        assert_eq!(
            (file_name.as_str(), folder.as_str()),
            ("SKILL.md", "review")
        );
        assert!(text.starts_with("---\nname:"));

        let ItemContent::Settings { verdict, .. } = &lib.item("s.yml").unwrap().content else {
            panic!("not settings");
        };
        // The verdict is the P4 hand-over check through the registry — here a fake third backend's refusal.
        assert_eq!(verdict.problems.len(), 1);
        assert_eq!(verdict.problems[0].0, "gamma");
        assert_eq!(verdict.problems[0].1.code, "gamma-line-unsafe");
        assert_eq!(verdict.problems[0].1.line, Some(3));

        let reason = |rel: &str| match &lib.item(rel).unwrap().content {
            ItemContent::Broken { reason } => reason.code.clone(),
            other => panic!("{rel}: {other:?}"),
        };
        assert_eq!(reason("bad.yaml"), "yaml-syntax");
        assert_eq!(reason("typo.yaml"), "kind-unknown");
    }

    #[test]
    fn a_yaml_file_that_claims_a_kind_but_is_not_utf8_is_broken() {
        let mut lib = Lib::new();
        lib.store.file("p.yaml", "kind: preset\nversion: 1\n");
        lib.scan(None);
        // Replace the text by bytes that are not UTF-8 (a Latin-1 'é' in the description).
        let path = lp("p.yaml").to_path(&lib.root);
        lib.store.remove("p.yaml");
        lib.store
            .write_atomic(&path, b"kind: preset\nversion: 1\ndescription: caf\xe9\n")
            .unwrap();
        lib.scan(None);
        let item = lib.item("p.yaml").unwrap();
        assert_eq!(
            item.kind,
            ItemKind::Broken {
                claimed: Some(Sniff::Preset)
            }
        );
        let ItemContent::Broken { reason } = &item.content else {
            panic!()
        };
        assert_eq!(reason.code, "yaml-syntax");
    }

    #[test]
    fn the_skill_file_name_follows_the_os_rule() {
        let mut lib = Lib::new();
        lib.store.file("skills/x/skill.md", "---\nname: x\n---\n");
        lib.scan(None);
        if same_path(Path::new("skill.md"), Path::new("SKILL.md")) {
            assert_eq!(lib.item("skills/x").unwrap().kind, ItemKind::Skill);
            assert_eq!(
                lib.item("skills/x/skill.md").unwrap().kind,
                ItemKind::SkillFile
            );
        } else {
            assert!(lib.item("skills/x").is_none());
            assert_eq!(
                lib.item("skills/x/skill.md").unwrap().kind,
                ItemKind::Instruction
            );
        }
    }

    #[test]
    fn a_skill_whose_skill_md_goes_becomes_a_plain_folder() {
        let mut lib = Lib::new();
        lib.store.file("skills/x/SKILL.md", "---\nname: x\n---\n");
        lib.store.file("skills/x/notes.md", "n");
        lib.scan(None);
        assert!(lib.item("skills/x").is_some());
        lib.store.remove("skills/x/SKILL.md");
        lib.scan(None);
        assert!(
            lib.item("skills/x").is_none(),
            "the folder is no longer a skill"
        );
        assert!(lib.item("skills/x/SKILL.md").is_none());
        assert!(lib.item("skills/x/notes.md").is_some());
        assert!(lib
            .catalog
            .folders()
            .any(|folder| folder.as_str() == "skills/x"));
    }

    // ── re-reads ──────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_scan_result_carries_fresh_and_unchanged_entries_and_listed_at_without_a_catalog() {
        let mut lib = Lib::new();
        lib.store.file("rules/a.md", "a");
        lib.store.file("rules/b.md", "b");
        lib.store.file("skills/s/SKILL.md", "---\nname: s\n---\n");
        let first = lib.build(None);
        assert_eq!(
            fresh_paths(&first),
            ["rules/a.md", "rules/b.md", "skills/s", "skills/s/SKILL.md"]
        );
        assert!(first.listed_at.contains_key(&None));
        assert!(first.listed_at.contains_key(&Some(lp("rules"))));
        lib.apply(first);

        lib.store.file_version("rules/b.md", "b2", 5);
        let second = lib.build(None);
        assert_eq!(
            fresh_paths(&second),
            ["rules/b.md"],
            "only the changed tag is re-read"
        );
        assert_eq!(
            unchanged_count(&second),
            3,
            "a.md, the skill and its SKILL.md"
        );
    }

    #[test]
    fn a_racy_item_is_re_read_although_its_tag_is_equal() {
        let mut lib = Lib::new();
        lib.store
            .file_at("racy.md", "r", now() - Duration::from_secs(1));
        lib.store
            .file_at("settled.md", "s", now() - Duration::from_secs(3));
        lib.scan(None);
        let tags = lib.catalog.tags(None);
        assert!(tags[&key(&lp("racy.md"))].1);
        assert!(!tags[&key(&lp("settled.md"))].1);
        let again = lib.build(None);
        assert_eq!(fresh_paths(&again), ["racy.md"]);
        assert_eq!(unchanged_count(&again), 1);
    }

    #[test]
    fn a_fresh_entry_older_than_the_current_item_does_not_overwrite_it() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "old text");
        let older = lib.build(None);
        lib.store.file_version("a.md", "new text", 2);
        lib.scan(None);
        let newer_seq = lib.item("a.md").unwrap().read_seq;
        let touched = lib.apply(older);
        assert!(touched.is_empty(), "{touched:?}");
        assert_eq!(lib.excerpt_of("a.md"), "new text");
        assert_eq!(lib.item("a.md").unwrap().read_seq, newer_seq);
    }

    #[test]
    fn an_unchanged_entry_writes_nothing() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "on disk");
        lib.scan(None);
        // A later observation with the same tag but other content (a stand-in for a merge the scan must not undo).
        let mut marker = lib.item("a.md").unwrap().clone();
        marker.read_seq = 1_000;
        marker.content = ItemContent::Instruction {
            excerpt: "marker".to_owned(),
        };
        lib.catalog.apply_items(vec![marker], false);
        // A merge is marked racy (re-read by the next scan); here it has settled, so the scan may skip it.
        lib.catalog.racy.remove(&key(&lp("a.md")));
        let result = lib.build(None);
        assert_eq!(unchanged_count(&result), 1);
        let touched = lib.apply(result);
        assert!(touched.is_empty());
        assert_eq!(lib.excerpt_of("a.md"), "marker");
        assert_eq!(lib.item("a.md").unwrap().read_seq, 1_000);
    }

    #[test]
    fn apply_items_keeps_the_later_read_and_inserts_only_when_allowed() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "a");
        lib.scan(None);
        let mut older = lib.item("a.md").unwrap().clone();
        older.read_seq = 0;
        older.content = ItemContent::Instruction {
            excerpt: "older".to_owned(),
        };
        assert!(lib.catalog.apply_items(vec![older], true).is_empty());
        assert_eq!(lib.excerpt_of("a.md"), "a");

        let mut new_path = lib.item("a.md").unwrap().clone();
        new_path.path = lp("b.md");
        new_path.read_seq = 99;
        assert!(lib
            .catalog
            .apply_items(vec![new_path.clone()], false)
            .is_empty());
        assert!(lib.item("b.md").is_none());
        assert_eq!(
            lib.catalog.apply_items(vec![new_path], true),
            [key(&lp("b.md"))]
        );
        assert!(lib.item("b.md").is_some());
    }

    // ── sequenced absences ────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn an_absent_item_read_after_its_folders_listing_survives_and_an_older_one_goes() {
        let mut lib = Lib::new();
        lib.store.file("rules/old.md", "o");
        lib.scan(None);
        lib.store.remove("rules/old.md");
        let result = lib.build(None);
        let listed = result.listed_at[&Some(lp("rules"))];
        // A file created after the listing, read by a launch check later than the listing.
        lib.store.file("rules/new.md", "n");
        lib.check_reads("rules/new.md");
        assert!(lib.item("rules/new.md").unwrap().read_seq > listed);
        lib.apply(result);
        assert!(
            lib.item("rules/old.md").is_none(),
            "read before the listing — removed"
        );
        assert!(
            lib.item("rules/new.md").is_some(),
            "read after the listing — kept"
        );
    }

    #[test]
    fn items_outside_a_partial_scans_range_are_untouched() {
        let mut lib = Lib::new();
        lib.store.file("a/x.md", "x");
        lib.store.file("b/y.md", "y");
        lib.scan(None);
        lib.store.remove("b/y.md");
        lib.store.remove("a/x.md");
        lib.scan(Some("a"));
        assert!(lib.item("a/x.md").is_none());
        assert!(lib.item("b/y.md").is_some(), "outside the range");
    }

    #[test]
    fn a_folder_whose_listing_fails_keeps_its_items() {
        let mut lib = Lib::new();
        lib.store.file("locked/c.md", "c");
        lib.store.file("open/d.md", "d");
        lib.scan(None);
        lib.store.fail_listing("locked", ListFailure::Open);
        lib.store.remove("open/d.md");
        let result = lib.build(None);
        assert_eq!(result.kept, [Some(lp("locked"))]);
        assert_eq!(result.errors.len(), 1);
        lib.apply(result);
        assert!(lib.item("locked/c.md").is_some(), "unseen, not absent");
        assert!(lib.item("open/d.md").is_none());
    }

    #[test]
    fn a_part_way_enumeration_failure_takes_what_was_listed_and_keeps_the_rest() {
        let mut lib = Lib::new();
        lib.store.file("half/1.md", "one");
        lib.store.file("half/2.md", "two");
        lib.store.file("half/sub/3.md", "three");
        lib.scan(None);
        lib.store.file_version("half/1.md", "one again", 2);
        lib.store
            .fail_listing("half", ListFailure::PartWay { after: 1 });
        lib.scan(None);
        assert_eq!(
            lib.excerpt_of("half/1.md"),
            "one again",
            "listed before the failure — taken"
        );
        assert!(lib.item("half/2.md").is_some(), "not listed — kept");
        assert!(
            lib.item("half/sub/3.md").is_some(),
            "below the failure — kept"
        );
    }

    #[test]
    fn a_file_whose_metadata_or_read_fails_keeps_its_item() {
        let mut lib = Lib::new();
        lib.store.file("meta.md", "m");
        lib.store.file("read.md", "r");
        lib.store.file("skills/s/SKILL.md", "---\nname: s\n---\n");
        lib.store.file("skills/s/gone.md", "g");
        lib.scan(None);
        lib.store.fail_tag("meta.md");
        lib.store.file_version("read.md", "r2", 2);
        lib.store.fail_read("read.md");
        lib.store.fail_tag("skills/s/SKILL.md");
        lib.store.remove("skills/s/gone.md");
        let result = lib.build(None);
        let mut kept: Vec<_> = result.kept.clone();
        kept.sort();
        assert_eq!(
            kept,
            [
                Some(lp("meta.md")),
                Some(lp("read.md")),
                Some(lp("skills/s/SKILL.md"))
            ]
        );
        lib.apply(result);
        assert!(lib.item("meta.md").is_some());
        assert_eq!(lib.excerpt_of("read.md"), "r", "the previous item stays");
        assert!(lib.item("skills/s/SKILL.md").is_some());
        assert!(
            lib.item("skills/s").is_some(),
            "the skill read from that file stays too"
        );
        assert!(
            lib.item("skills/s/gone.md").is_none(),
            "its folder was listed — not kept"
        );
    }

    #[test]
    fn a_partial_scan_under_an_internal_link_removes_older_items_and_keeps_a_newer_check() {
        let mut lib = Lib::new();
        lib.store.file("team/a.md", "a");
        lib.store.file("team/b.md", "b");
        lib.scan(None);
        lib.store.link("team");
        let result = lib.build(Some("team"));
        assert_eq!(result.skipped_links, [lp("team")]);
        let (range, absent_at) = result.absent_ranges[0].clone();
        assert_eq!(range, lp("team"));
        // The link is replaced by a real folder again and a launch check reads one of its files — later than the
        //   number drawn before the link check.
        lib.store.remove("team");
        lib.store.file("team/b.md", "b again");
        lib.check_reads("team/b.md");
        assert!(lib.item("team/b.md").unwrap().read_seq > absent_at);
        lib.apply(result);
        assert!(
            lib.item("team/a.md").is_none(),
            "read before the absence — removed"
        );
        assert_eq!(
            lib.excerpt_of("team/b.md"),
            "b again",
            "read after it — kept"
        );
    }

    #[test]
    fn a_partial_scan_of_a_deleted_range_removes_the_range() {
        let mut lib = Lib::new();
        lib.store.file("team/a.md", "a");
        lib.store.file("team/deep/b.md", "b");
        lib.store.file("other.md", "o");
        lib.scan(None);
        lib.store.remove("team");
        let result = lib.build(Some("team"));
        assert_eq!(result.absent_ranges.len(), 1);
        lib.apply(result);
        assert!(lib.item("team/a.md").is_none());
        assert!(lib.item("team/deep/b.md").is_none());
        assert!(lib.item("other.md").is_some());
        assert!(!lib
            .catalog
            .folders()
            .any(|folder| folder.as_str().starts_with("team")));
    }

    #[test]
    fn remove_refuses_a_number_not_later_than_the_read() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "a");
        lib.scan(None);
        let k = key(&lp("a.md"));
        let read = lib.item("a.md").unwrap().read_seq;
        assert!(!lib.catalog.remove(&k, read));
        assert!(lib.catalog.remove(&k, read + 1));
        assert!(lib.catalog.get(&k).is_none());
    }

    #[test]
    fn children_lists_the_items_directly_in_a_folder() {
        let mut lib = Lib::new();
        lib.store.file("top.md", "t");
        lib.store.file("rules/b.md", "b");
        lib.store.file("rules/a.md", "a");
        lib.store.file("rules/deep/c.md", "c");
        lib.scan(None);
        let names = |folder: Option<&str>| -> Vec<String> {
            let folder = folder.map(lp);
            lib.catalog
                .children(folder.as_ref())
                .into_iter()
                .map(|item| item.path.as_str().to_owned())
                .collect()
        };
        assert_eq!(names(None), ["top.md"]);
        assert_eq!(names(Some("rules")), ["rules/a.md", "rules/b.md"]);
    }

    // ── derived data ──────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn recompute_updates_used_by_for_touched_presets_and_presets_reaching_them() {
        let mut lib = Lib::new();
        lib.store.file("rules/a.md", "a");
        lib.store.file("rules/b.md", "b");
        lib.store.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - mid.yaml\n",
        );
        lib.store.file(
            "mid.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/a.md\n",
        );
        lib.scan(None);
        assert_eq!(lib.reaching("rules/a.md"), ["mid.yaml", "top.yaml"]);
        lib.store.file_version(
            "mid.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/b.md\n",
            2,
        );
        let touched = lib.scan(None);
        assert_eq!(
            touched,
            [key(&lp("mid.yaml"))],
            "top.yaml itself is unchanged"
        );
        assert!(lib.reaching("rules/a.md").is_empty());
        assert_eq!(
            lib.reaching("rules/b.md"),
            ["mid.yaml", "top.yaml"],
            "top reaches the touched mid"
        );
    }

    #[test]
    fn a_preset_cycle_recomputes_without_looping() {
        let mut lib = Lib::new();
        lib.store.file("x.md", "x");
        lib.store
            .file("a.yaml", "kind: preset\nversion: 1\npresets:\n  - b.yaml\n");
        lib.store.file(
            "b.yaml",
            "kind: preset\nversion: 1\npresets:\n  - a.yaml\ninstructions:\n  - x.md\n",
        );
        lib.scan(None);
        assert_eq!(lib.reaching("x.md"), ["a.yaml", "b.yaml"]);
        assert_eq!(
            lib.reaching("a.yaml"),
            ["b.yaml"],
            "a preset does not reach itself"
        );
        assert_eq!(lib.reaching("b.yaml"), ["a.yaml"]);
    }

    #[test]
    fn the_reverse_index_keeps_missing_targets_so_an_inserted_child_is_reached_at_once() {
        let mut lib = Lib::new();
        lib.store.file("x.md", "x");
        lib.store.file(
            "top.yaml",
            "kind: preset\nversion: 1\npresets:\n  - child.yaml\n",
        );
        lib.scan(None);
        assert!(
            lib.catalog.refs_to.contains_key(&key(&lp("child.yaml"))),
            "missing target kept"
        );
        lib.store.file(
            "child.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - x.md\n",
        );
        let touched = lib.scan(None);
        assert_eq!(touched, [key(&lp("child.yaml"))]);
        assert_eq!(lib.reaching("x.md"), ["child.yaml", "top.yaml"]);
    }

    #[test]
    fn a_merged_item_is_marked_racy_and_re_read_by_the_next_scan() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "a");
        lib.scan(None);
        assert!(!lib.catalog.tags(None)[&key(&lp("a.md"))].1);
        lib.check_reads("a.md");
        assert!(
            lib.catalog.tags(None)[&key(&lp("a.md"))].1,
            "an old file, but merged"
        );
        assert_eq!(fresh_paths(&lib.build(None)), ["a.md"]);
    }

    #[test]
    fn the_racy_window_includes_its_edge() {
        let mut lib = Lib::new();
        lib.store.file_at("edge.md", "e", now() - RACY_WINDOW);
        lib.store.file_at(
            "past.md",
            "p",
            now() - RACY_WINDOW - Duration::from_millis(1),
        );
        lib.scan(None);
        let tags = lib.catalog.tags(None);
        assert!(
            tags[&key(&lp("edge.md"))].1,
            "mtime == scan time - 2 s is racy"
        );
        assert!(!tags[&key(&lp("past.md"))].1);
    }

    #[test]
    fn an_item_with_the_same_read_seq_does_not_replace_the_current_one() {
        let mut lib = Lib::new();
        lib.store.file("a.md", "a");
        lib.scan(None);
        let mut same = lib.item("a.md").unwrap().clone();
        same.content = ItemContent::Instruction {
            excerpt: "same number".to_owned(),
        };
        assert!(lib.catalog.apply_items(vec![same], true).is_empty());
        assert_eq!(lib.excerpt_of("a.md"), "a");
    }

    #[test]
    fn a_folder_whose_own_read_failed_is_kept_although_its_parent_was_listed() {
        let mut lib = Lib::new();
        lib.store.file("f/x.md", "x");
        lib.scan(None);
        // A scan that listed the root, did not list `f` and charged a failure to `f` itself.
        let listing = ScanListing {
            root: lib.root.clone(),
            listed_at: HashMap::from([(None, lib.seq())]),
            errors: vec![(Some(lp("f")), "injected".to_owned())],
            ..ScanListing::default()
        };
        let tags = lib.catalog.tags(None);
        let result = ScanResult::build(
            listing,
            None,
            &tags,
            &lib.store.reader(),
            &lib.backends,
            now(),
        );
        lib.apply(result);
        assert!(lib.catalog.folders().any(|folder| folder.as_str() == "f"));
        assert!(lib.item("f/x.md").is_some());
    }

    #[test]
    fn a_deep_folder_carries_its_own_listing_number_not_the_scans_first() {
        use std::cell::{Cell, RefCell};
        let mut lib = Lib::new();
        lib.store.file("a/1.md", "1");
        lib.store.file("a/2.md", "2");
        lib.store.file("d/x.md", "x");
        lib.scan(None);
        // Steps: root, a, a/1.md, a/2.md, leaving a, d … — call 5 comes before the step that lists d. There, during
        //   the walk, a launch check reads d/x.md and then d/x.md is deleted.
        let calls = Cell::new(0);
        let check: RefCell<Option<Item>> = RefCell::new(None);
        let seq = || {
            calls.set(calls.get() + 1);
            if calls.get() == 5 {
                *check.borrow_mut() = Some(lib.overlay_read("d/x.md"));
                lib.store.remove("d/x.md");
            }
            lib.seq()
        };
        let listing = lib.store.scan(&lib.root, None, &seq).unwrap();
        let tags = lib.catalog.tags(None);
        let result = ScanResult::build(
            listing,
            None,
            &tags,
            &lib.store.reader(),
            &lib.backends,
            now(),
        );
        let check = check.into_inner().unwrap();
        // One number per scan would be the root's — earlier than the check's read, so the check's item would stay.
        assert!(result.listed_at[&None] < check.read_seq);
        assert!(result.listed_at[&Some(lp("d"))] > check.read_seq);
        // The check's merge lands first, then the scan.
        lib.catalog.apply_items(vec![check], true);
        lib.apply(result);
        assert!(
            lib.item("d/x.md").is_none(),
            "d was listed after the check's read"
        );
    }

    #[test]
    fn the_preset_side_reverse_index_follows_edits_and_removals() {
        let mut lib = Lib::new();
        lib.store.file("x.md", "x");
        lib.store.file(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - x.md\n  - engram:/priming\n",
        );
        lib.scan(None);
        let refs = |lib: &Lib, rel: &str| {
            lib.catalog
                .refs_to
                .get(&key(&LibPath::parse(rel).unwrap()))
                .cloned()
        };
        assert_eq!(
            refs(&lib, "x.md"),
            Some(BTreeSet::from([key(&lp("p.yaml"))]))
        );
        assert_eq!(
            lib.catalog.refs_to.len(),
            1,
            "a built-in is not a library reference"
        );
        // An edit that lists another target moves the edge.
        lib.store.file_version(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - y.md\n",
            2,
        );
        lib.scan(None);
        assert_eq!(refs(&lib, "x.md"), None);
        assert_eq!(
            refs(&lib, "y.md"),
            Some(BTreeSet::from([key(&lp("p.yaml"))])),
            "a missing target keeps its edge"
        );
        lib.store.remove("p.yaml");
        lib.scan(None);
        assert_eq!(
            refs(&lib, "y.md"),
            None,
            "a removed preset takes its edges along"
        );
        assert!(lib.catalog.refs_to.is_empty());
        assert!(lib.catalog.used_by.is_empty());
    }

    #[test]
    fn clear_empties_items_folders_tags_used_by_and_refs_to() {
        let mut lib = Lib::new();
        lib.store.file_at("rules/racy.md", "r", now());
        lib.store.file(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/racy.md\n",
        );
        lib.scan(None);
        assert!(!lib.catalog.racy.is_empty());
        assert!(!lib.catalog.used_by.is_empty());
        lib.catalog.clear();
        assert!(lib.catalog.items.is_empty());
        assert!(lib.catalog.tags(None).is_empty());
        assert_eq!(lib.catalog.folders().count(), 0);
        assert!(lib.catalog.used_by.is_empty());
        assert!(lib.catalog.refs_to.is_empty());
        assert!(lib.catalog.racy.is_empty());
    }
}

/// The scan cost on a generated library (TRD S25 step-7 R3 · §6-2 L1004) — a measure, not a check: it prints the
/// times and asserts only the counts. Run with `cargo test -p engram-dashboard-asset-library scan_cost -- --ignored
/// --nocapture`.
#[cfg(test)]
mod measure {
    use std::cell::Cell;
    use std::fs;
    use std::time::Instant;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::store::{FsStore, LibraryStore};

    #[test]
    #[ignore = "measure — run by hand"]
    fn scan_cost_of_a_5000_item_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 100 folders × 50 files: per folder 30 instructions, 10 presets, 5 settings, 5 skills (folder + SKILL.md).
        let mut files = 0;
        for f in 0..100 {
            let folder = root.join(format!("f{f:03}"));
            fs::create_dir_all(&folder).unwrap();
            for i in 0..30 {
                let text = format!(
                    "# Rule {i}\n\nFirst paragraph of rule {i} in folder {f}.\n\n```\ncode\n```\n"
                );
                fs::write(folder.join(format!("r{i:02}.md")), text).unwrap();
            }
            for i in 0..10 {
                let text = format!(
                    "kind: preset\nversion: 1\ndescription: preset {i}\ninstructions:\n  - f{f:03}/r{i:02}.md\n  - f{f:03}/r{:02}.md\nskills:\n  - f{f:03}/s{:01}\nsettings:\n  - f{f:03}/set{:01}.yaml\n",
                    i + 10,
                    i % 5,
                    i % 5
                );
                fs::write(folder.join(format!("p{i:02}.yaml")), text).unwrap();
            }
            for i in 0..5 {
                let text = format!(
                    "kind: settings\nversion: 1\nclaude:\n  model: m{i}\ncodex:\n  - model=m{i}\n"
                );
                fs::write(folder.join(format!("set{i}.yaml")), text).unwrap();
                let skill = folder.join(format!("s{i}"));
                fs::create_dir_all(&skill).unwrap();
                let text = format!("---\nname: skill {f}-{i}\ndescription: does {i}\n---\nbody\n");
                fs::write(skill.join("SKILL.md"), text).unwrap();
            }
            files += 50;
        }
        assert_eq!(files, 5_000);

        let next = Cell::new(0u64);
        let seq = || {
            next.set(next.get() + 1);
            next.get()
        };
        let read = |path: &Path| FsStore.read(path, 1 << 20);
        let mut catalog = Catalog::default();

        let started = Instant::now();
        let scan_time = SystemTime::now();
        let listing = FsStore.scan(root, None, &seq).unwrap();
        let walked = started.elapsed();
        assert_eq!(listing.files.len(), 5_000);
        let result = ScanResult::build(
            listing,
            None,
            &catalog.tags(None),
            &read,
            &AgentBackends,
            scan_time,
        );
        let built = started.elapsed();
        let touched = catalog.apply_scan(result);
        catalog.recompute_derived(&touched);
        let first = started.elapsed();
        // 5,000 files make 5,500 items (each skill is a folder item and its SKILL.md item).
        assert_eq!(touched.len(), 5_500);

        // The files were just written, so the first scan marked them racy. Let the racy window pass, so the second
        //   scan re-reads them once and clears the marks.
        std::thread::sleep(RACY_WINDOW + Duration::from_secs(1));
        let again = Instant::now();
        let later = SystemTime::now();
        let listing = FsStore.scan(root, None, &seq).unwrap();
        let rewalked = again.elapsed();
        let result = ScanResult::build(
            listing,
            None,
            &catalog.tags(None),
            &read,
            &AgentBackends,
            later,
        );
        let fresh = result
            .entries
            .iter()
            .filter(|entry| matches!(entry, ScanItem::Fresh(_)))
            .count();
        let touched = catalog.apply_scan(result);
        catalog.recompute_derived(&touched);
        let second = again.elapsed();

        // The second scan re-read the files the first one marked racy and cleared the marks; a third is steady state.
        let steady = Instant::now();
        let later = SystemTime::now();
        let listing = FsStore.scan(root, None, &seq).unwrap();
        let result = ScanResult::build(
            listing,
            None,
            &catalog.tags(None),
            &read,
            &AgentBackends,
            later,
        );
        let steady_fresh = result
            .entries
            .iter()
            .filter(|entry| matches!(entry, ScanItem::Fresh(_)))
            .count();
        let touched = catalog.apply_scan(result);
        catalog.recompute_derived(&touched);
        let third = steady.elapsed();
        assert_eq!(steady_fresh, 0);

        println!(
            "scan cost, 5,000 files: first scan walk+tag {walked:?}, +read+parse {built:?}, +apply+derive {first:?}; \
             second scan walk+tag {rewalked:?}, total {second:?} (re-read {fresh} racy); third scan total {third:?} \
             (re-read {steady_fresh})"
        );
    }
}
