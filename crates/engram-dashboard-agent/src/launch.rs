//! The launch-material port — the path by which the manager takes the materials from attached presets (instructions · skills · raw settings) and puts them on a spawn.
//!
//! This file holds: the material source [`LaunchKitSource`] and its default implementation [`NoopLaunchKits`] · [`LaunchFs`], through
//! which backend placement writes to disk, with its production implementation [`StdLaunchFs`] · `place_in_volatile`, which confines
//! placement to the volatile folder · the placement result [`LaunchArtifacts`] · `LaunchKitGuard`, which reclaims materials when a spawn
//! fails · `LaunchLease`, with which the reaper reclaims a launched incarnation's materials · [`LaunchPorts`], the bundle the manager takes.
//! Preset resolution is not here (the real one = the asset-library service the daemon plugs in). Neither is flag translation (the backend's job).
// ADR-0334

use std::any::Any;
use std::cmp::Ordering;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use engram_dashboard_platform::fs::same_path;
use serde_json::{Map, Number, Value};

use crate::profile::{AgentCommand, AgentProfile};
use crate::types::AgentId;

// ── plan · materials ───────────────────────────────────────────────────────────────

/// One reason — the machine-readable code, its arguments and the line hit. Block reasons and notices share this shape.
///
/// `line` = the line of the asset file hit — counted from 1 (first line = 1). `None` = a reason that cannot point at a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: String,
    pub args: Value,
    pub line: Option<u32>,
}

/// A notice carried in a launch success answer — the shape is [`Reason`] as is.
pub type Notice = Reason;

// `Value` has no `Ord`, so it is written by hand — `BlockReason`'s `Ord` rides on it. The order agrees with `==`
//   (equal ⟺ `==`) — if it did not, sorting could panic.
impl Ord for Reason {
    fn cmp(&self, other: &Self) -> Ordering {
        self.code
            .cmp(&other.code)
            .then_with(|| cmp_json(&self.args, &other.args))
            .then_with(|| self.line.cmp(&other.line))
    }
}

impl PartialOrd for Reason {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Objects compare by key name order — `Value`'s `==` ignores key order, and if the `preserve_order` feature is turned on in another
//   crate of the same build, iteration order becomes insertion order.
fn cmp_json(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => cmp_number(x, y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = cmp_json(p, q);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut xs: Vec<_> = x.iter().collect();
            let mut ys: Vec<_> = y.iter().collect();
            xs.sort_by(|p, q| p.0.cmp(q.0));
            ys.sort_by(|p, q| p.0.cmp(q.0));
            for ((pk, pv), (qk, qv)) in xs.iter().zip(&ys) {
                let o = pk.cmp(qk).then_with(|| cmp_json(pv, qv));
                if o != Ordering::Equal {
                    return o;
                }
            }
            xs.len().cmp(&ys.len())
        }
        _ => rank(a).cmp(&rank(b)),
    }
}

// Two numbers that are `==` are equal (-0.0 and 0.0 included). Otherwise by (kind, value), and if still undecided by text — the text
//   comparison is reached only when the `arbitrary_precision` feature is on and `==` compares text.
fn cmp_number(a: &Number, b: &Number) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    fn kind(n: &Number) -> u8 {
        if n.as_u64().is_some() {
            1
        } else if n.as_i64().is_some() {
            0
        } else {
            2
        }
    }
    let by_value = match (kind(a), kind(b)) {
        (0, 0) => a.as_i64().cmp(&b.as_i64()),
        (1, 1) => a.as_u64().cmp(&b.as_u64()),
        (2, 2) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(Ordering::Equal),
        (x, y) => x.cmp(&y),
    };
    by_value.then_with(|| a.to_string().cmp(&b.to_string()))
}

/// One instructions source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// Where the source comes from, in one of three forms: a library path from the root (`/`-separated), a built-in
    /// `engram:/…` reference, or for a folder entry `folder:` + the normalized absolute folder path (e.g.
    /// `folder:D:\rules\company`) — then [`SourceRef::file`] is the instruction file chosen inside that folder.
    /// A `folder:` string must not be matched against a library path (a folder inside the library is still a folder).
    // ADR-0341
    pub path: String,
    /// The absolute path of that file.
    pub file: PathBuf,
}

/// One skill to carry — its name and absolute folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRef {
    pub name: String,
    pub dir: PathBuf,
}

/// One block of that backend's raw settings — `preset` = the reference of the preset that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSetting {
    pub preset: String,
    pub value: Value,
}

/// The launch plan [`LaunchKitSource::check`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The agent backend word — the serde `kind` tag of [`AgentCommand`] in lowercase (`claude` · `codex` ·
    /// `shell`). Every [`LaunchKitSource`] implementation fills it only through [`backend_word`] — building it per implementation would
    /// split one backend into words differing in case · spelling.
    pub backend: String,
    /// Instructions sources — in merge order.
    pub instructions: Vec<SourceRef>,
    pub skills: Vec<SkillRef>,
    /// That backend's raw settings — in attach order. Model · effort go here too (there is no separate field).
    pub raw_settings: Vec<RawSetting>,
    /// Whether the merged instructions take 「Default」's priming (the shipped `builtin\`). Meaningful only when
    /// [`LaunchPlan::real`] — otherwise nobody reads it, and the launch carries no priming at all (ADR-0370).
    pub include_priming: bool,
    pub notices: Vec<Notice>,
    /// Real materials? `false` → the backend translation carries no material at all (the Noop argv golden,
    /// `tests/launch_argv_golden.rs`).
    pub real: bool,
    /// Folder entries — absolute normalized paths in expansion order, deduplicated. Only `backend/claude` reads them
    /// (the backend loads each folder itself). Empty in a Noop plan.
    // ADR-0341
    pub folders: Vec<PathBuf>,
    /// Extra skill roots that codex JSON appends to its skill roots as they are (the folder entries' `.agents/skills`
    /// and `.codex/skills` that exist). Roots, not named skills — they join neither [`LaunchPlan::skills`] nor the
    /// conflict check. Only `backend/codex` reads them. Empty in a Noop plan.
    // ADR-0341
    pub extra_skill_roots: Vec<PathBuf>,
    /// What the check read of the plan's sources, for [`LaunchKitSource::prepare`] to tell a changed source
    /// ([`PrepareError::Stale`]). Opaque to everyone else — the source that wrote it reads it. `None` in a Noop plan.
    pub stamp: Option<String>,
}

/// This incarnation's materials as [`LaunchKitSource::prepare`] produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchKit {
    pub plan: LaunchPlan,
    /// This incarnation's volatile folder — absolute path. `None` = no folder was made (backend placement can write nothing).
    /// Backend placement writes only inside this folder (`place_in_volatile` enforces it — a relative path fails the placement).
    pub volatile_dir: Option<PathBuf>,
    /// The merged instructions file. `None` = no instructions to carry.
    pub instructions_file: Option<PathBuf>,
    /// Skills to carry — absolute folders.
    pub skills: Vec<SkillRef>,
    /// The hash of the merged instructions — codex JSON resume compares it with the last applied value. `None` = no instructions to carry.
    pub instructions_hash: Option<String>,
    /// The hash of the instructions the agent's conversation last received
    /// ([`AgentProfile::last_applied_instructions_hash`]). [`LaunchKitSource::prepare`] leaves it `None`; the manager
    /// fills it from the registry re-read right after `prepare`. Read only by `backend/codex` (the resume context).
    // ADR-0337
    pub last_applied_instructions_hash: Option<String>,
}

// ── blocks ──────────────────────────────────────────────────────────────────────

/// One reason that blocks a launch. Variant order is display order — `Ord` sorts by it (TRD S25 §1-3 8).
///
/// `item` = the blocked item: its library path, or for a folder entry `folder:` + the normalized absolute folder path
/// (the same third form as [`SourceRef::path`]). A row about no library item names what it is about with a kind tag
/// in the same way: `command-line:` + the backend word (the command line does not fit) and `working-folder:` + the
/// working folder's path (the backend refuses that folder). `via` = the presets that reference it (the chain it came
/// through) — empty for the two tagged rows.
// ADR-0341
// ADR-0361
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockReason {
    /// The attached file is missing.
    Missing {
        item: String,
        via: Vec<String>,
        reason: Reason,
    },
    /// Cannot be read or handed over — YAML syntax · a cycle · a skill codex cannot load · the command-line guard ·
    /// a command line over its length.
    Broken {
        item: String,
        via: Vec<String>,
        reason: Reason,
    },
    /// One conflict — `items` = (path, route) per item hit.
    Conflict {
        reason: Reason,
        items: Vec<(String, Vec<String>)>,
    },
    /// A library preset is attached while the library folder is unset or missing.
    LibraryUnavailable { reason: Reason },
}

/// [`LaunchKitSource::check`]'s refusal — every reason hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchBlocked {
    pub reasons: Vec<BlockReason>,
}

impl fmt::Display for LaunchBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("launch blocked: ")?;
        if self.reasons.is_empty() {
            return f.write_str("(no reason)");
        }
        for (i, r) in self.reasons.iter().enumerate() {
            if i > 0 {
                f.write_str(" · ")?;
            }
            match r {
                BlockReason::Missing { item, reason, .. } => {
                    write!(f, "not found {} ({})", escape_controls(item), reason.code)?
                }
                BlockReason::Broken { item, reason, .. } => {
                    write!(f, "error {} ({})", escape_controls(item), reason.code)?
                }
                BlockReason::Conflict { reason, items } => {
                    let paths: Vec<String> =
                        items.iter().map(|(p, _)| escape_controls(p)).collect();
                    write!(f, "conflict {} ({})", paths.join(", "), reason.code)?
                }
                BlockReason::LibraryUnavailable { reason } => {
                    write!(f, "library folder missing ({})", reason.code)?
                }
            }
        }
        Ok(())
    }
}

impl std::error::Error for LaunchBlocked {}

// ── Reply rows ────────────────────────────────────────────────────────────────

impl LaunchBlocked {
    /// The reasons as `CommandError` args: `{"reasons": [row, …]}`, rows in the stored order. A row by kind:
    /// - `{"kind": "missing" | "broken", "item", "via": [..], "code", "args", "line"?}`
    /// - `{"kind": "conflict", "items": [{"item", "via": [..]}, ..], "code", "args", "line"?}`
    /// - `{"kind": "library_unavailable", "code", "args", "line"?}`
    ///
    /// `line` is present only when the reason names one. A `Map`, not a `Value`, so a caller can put keys beside
    /// `reasons` (an after-create block adds the created agent's id).
    pub fn to_args(&self) -> Map<String, Value> {
        let reasons = self
            .reasons
            .iter()
            .map(|r| {
                let mut row = Map::new();
                let reason = match r {
                    BlockReason::Missing { item, via, reason }
                    | BlockReason::Broken { item, via, reason } => {
                        let kind = if matches!(r, BlockReason::Missing { .. }) {
                            "missing"
                        } else {
                            "broken"
                        };
                        row.insert("kind".into(), kind.into());
                        row.insert("item".into(), item.as_str().into());
                        row.insert("via".into(), via.clone().into());
                        reason
                    }
                    BlockReason::Conflict { reason, items } => {
                        row.insert("kind".into(), "conflict".into());
                        let items = items
                            .iter()
                            .map(|(item, via)| {
                                let mut entry = Map::new();
                                entry.insert("item".into(), item.as_str().into());
                                entry.insert("via".into(), via.clone().into());
                                Value::Object(entry)
                            })
                            .collect::<Vec<_>>();
                        row.insert("items".into(), items.into());
                        reason
                    }
                    BlockReason::LibraryUnavailable { reason } => {
                        row.insert("kind".into(), "library_unavailable".into());
                        reason
                    }
                };
                put_reason(&mut row, reason);
                Value::Object(row)
            })
            .collect::<Vec<_>>();
        let mut args = Map::new();
        args.insert("reasons".into(), reasons.into());
        args
    }
}

/// One notice as a reply row: `{"code", "args", "line"?, "text"}` — `line` only when the notice names one, `text` =
/// [`notice_text`]. The bus `agent.spawn` answer declares the same fields as a typed row and sends an absent `line` as
/// `null` (the command macro's `Option` has no skip) — a commands test pins the equality otherwise.
pub fn notice_row(notice: &Notice) -> Value {
    let mut row = Map::new();
    put_reason(&mut row, notice);
    row.insert("text".into(), notice_text(notice).into());
    Value::Object(row)
}

fn put_reason(row: &mut Map<String, Value>, reason: &Reason) {
    row.insert("code".into(), reason.code.as_str().into());
    row.insert("args".into(), reason.args.clone());
    if let Some(line) = reason.line {
        row.insert("line".into(), line.into());
    }
}

/// Notice code: codex terminal mode does not load the preset instructions and skills. `args` =
/// `{instructions: [library paths], skills: [names]}`.
// ADR-0337
pub const CODEX_TERMINAL_OMITTED: &str = "codex-terminal-omitted";

/// Notice code: codex terminal mode does not load a folder entry. `args` = `{folder, items: [relative paths]}`.
// ADR-0341
pub const CODEX_TERMINAL_FOLDER_OMITTED: &str = "codex-terminal-folder-omitted";

/// Notice code: skills of one name sit in several skill roots of a codex JSON agent, so `$name` selects none of them.
/// `args` = `{name, paths: [...]}`.
// ADR-0341
// ADR-0355
pub const CODEX_SAME_NAME_SKILLS: &str = "codex-same-name-skills";

/// Notice code: a folder entry's listing stopped at [`FOLDER_LISTING_CAP`] entries, so only part of the folder was
/// looked at. `args` = `{folder}`.
// ADR-0364
pub const FOLDER_LISTING_PARTIAL: &str = "folder-listing-partial";

/// How many entries a folder-entry listing reads before it stops — the one value the [`FOLDER_LISTING_PARTIAL`]
/// sentence uses and the asset-library folder listing is to use (step 7 P6).
pub const FOLDER_LISTING_CAP: usize = 10_000;

/// Notice code: subfolders or entries below a folder entry could not be read and were left out of its listing.
/// `args` = `{folder, paths: [folder-relative paths, walk order]}`.
// ADR-0363
// ADR-0364
pub const FOLDER_LISTING_SKIPPED: &str = "folder-listing-skipped";

/// How many of [`FOLDER_LISTING_SKIPPED`]'s paths its sentence names; the rest are counted.
const FOLDER_LISTING_SKIPPED_NAMED: usize = 3;

/// Notice code: a built-in asset could not be read, so the resolution applied an empty stand-in for it. `args` =
/// `{path, why}` — `path` = the built-in's spelling (`engram:/default`), `why` = why it could not be read.
// ADR-0361
// ADR-0364
pub const BUILTIN_UNAVAILABLE: &str = "builtin-unavailable";

/// The wording of a notice for a person or an LLM reading the reply — Korean, factual, recommends no fix (TRD S25
/// G28 · §2-4 8).
///
/// [`CODEX_TERMINAL_OMITTED`], [`CODEX_TERMINAL_FOLDER_OMITTED`] and [`CODEX_SAME_NAME_SKILLS`] have their own
/// sentence (user wording, exact), filled from `args` with names joined by `, `; in the two terminal codes an empty
/// list leaves its part out. [`FOLDER_LISTING_PARTIAL`] and [`FOLDER_LISTING_SKIPPED`] have their own sentence too,
/// naming no backend — the folder (and the skipped paths, the first three named and the rest counted) come from
/// `args`. [`BUILTIN_UNAVAILABLE`] has its own sentence, naming the built-in and why from `args`. When their `args` do
/// not have the shape the constant documents (not an object, a missing `folder`, `name`, `path` or `why`, a value
/// that is not a string, no `paths`), they fall back to the generic wording. Every other
/// code takes the generic wording — the code and the values listed in `args` (object keys in name order), saying the
/// omission is intended.
/// Control characters in the code and the listed values (file names from the library — outside input) are escaped
/// (`\n`, `\u{1b}`), so the line stays one line and carries no terminal escape when the CLI prints it.
// ADR-0364
pub fn notice_text(notice: &Notice) -> String {
    if let Some(text) = own_wording(notice) {
        return text;
    }
    let mut listed = Vec::new();
    collect_listed(&notice.args, &mut listed);
    let code = escape_controls(&notice.code);
    if listed.is_empty() {
        format!("Notice ({code}): intended behavior.")
    } else {
        format!(
            "Notice ({code}): intended behavior. Target — {}",
            listed.join(" · ")
        )
    }
}

/// The sentence of a code that has its own wording, or `None` (another code, or `args` of another shape).
// ADR-0337
// ADR-0341
// ADR-0355
// ADR-0364
fn own_wording(notice: &Notice) -> Option<String> {
    let args = notice.args.as_object()?;
    match notice.code.as_str() {
        CODEX_TERMINAL_OMITTED => {
            let mut parts = Vec::new();
            for (key, label) in [("instructions", "instructions"), ("skills", "skills")] {
                let names = text_list(args, key)?;
                if !names.is_empty() {
                    parts.push(format!("{label} {}", names.join(", ")));
                }
            }
            let head =
                "Notice: codex terminal mode does not support preset instructions and skills.";
            Some(if parts.is_empty() {
                head.to_string()
            } else {
                format!("{head} Not applied: {}", parts.join(", "))
            })
        }
        CODEX_TERMINAL_FOLDER_OMITTED => {
            let folder = escape_controls(args.get("folder")?.as_str()?);
            let items = text_list(args, "items")?;
            let head =
                "Notice: codex terminal mode does not support folder entries. Folder not applied:";
            Some(if items.is_empty() {
                format!("{head} {folder}")
            } else {
                format!("{head} {folder} ({})", items.join(", "))
            })
        }
        CODEX_SAME_NAME_SKILLS => {
            let name = escape_controls(args.get("name")?.as_str()?);
            let paths = text_list(args, "paths")?;
            if paths.is_empty() {
                return None;
            }
            Some(format!(
                "Notice: skills with the same name exist in several places. Calling ${name} in codex selects none of them. {name}: {}",
                paths.join(", ")
            ))
        }
        FOLDER_LISTING_PARTIAL => {
            let folder = escape_controls(args.get("folder")?.as_str()?);
            Some(format!(
                "Notice: folder {folder} has more than {} entries; only some were checked.",
                grouped(FOLDER_LISTING_CAP)
            ))
        }
        FOLDER_LISTING_SKIPPED => {
            let folder = escape_controls(args.get("folder")?.as_str()?);
            let paths = text_list(args, "paths")?;
            if paths.is_empty() {
                return None;
            }
            let named = paths.len().min(FOLDER_LISTING_SKIPPED_NAMED);
            let mut text = format!(
                "Notice: folder {folder}: skipped unreadable entries: {}",
                paths[..named].join(", ")
            );
            if paths.len() > named {
                text.push_str(&format!(" and {} more", grouped(paths.len() - named)));
            }
            Some(text)
        }
        BUILTIN_UNAVAILABLE => {
            let path = escape_controls(args.get("path")?.as_str()?);
            let why = escape_controls(args.get("why")?.as_str()?);
            Some(format!(
                "Notice: could not read built-in {path}; applied it with empty content. Reason: {why}"
            ))
        }
        _ => None,
    }
}

/// `n` in decimal with a `,` between groups of three digits (`10000` → `10,000`).
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The strings of the list at `key`, escaped — empty when the key is absent, `None` when it is not a list of strings.
fn text_list(args: &Map<String, Value>, key: &str) -> Option<Vec<String>> {
    match args.get(key) {
        None => Some(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(escape_controls))
            .collect(),
        Some(_) => None,
    }
}

/// `s` with every control character escaped (`\n`, `\u{1b}`) — outside text put into a line that is printed or logged.
pub fn escape_controls(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn collect_listed(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Null => {}
        Value::String(s) => out.push(escape_controls(s)),
        Value::Bool(_) | Value::Number(_) => out.push(value.to_string()),
        Value::Array(items) => items.iter().for_each(|v| collect_listed(v, out)),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            entries.iter().for_each(|(_, v)| collect_listed(v, out));
        }
    }
}

/// The omission identity of a notice set — what "the same notices" means for notices once per agent.
///
/// Per notice: `code` and `args` in a canonical form (object keys sorted · array elements sorted, so a reordered name
/// or path list is the same notice), `line` left out — a moved line or a reordered key does not make a new notice. The
/// set: those identities sorted, deduplicated and joined by `\n` (compact JSON escapes every newline in a string, so
/// the join cannot be forged by an argument). The empty set is the empty string.
/// The value is persisted as is (no hash — a hash of std is not stable across Rust releases) and only compared for
/// equality: the manager does not interpret it.
// ADR-0341
pub fn notice_fingerprint(notices: &[Notice]) -> String {
    let mut identities: Vec<String> = notices
        .iter()
        .map(|n| {
            format!(
                "[{},{}]",
                Value::String(n.code.clone()),
                canonical_json(&n.args)
            )
        })
        .collect();
    identities.sort();
    identities.dedup();
    identities.join("\n")
}

// Not `Value::to_string` — key order follows the map, and if `preserve_order` is ever enabled elsewhere in the build
//   that becomes insertion order.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let body: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}:{}", Value::String((*k).clone()), canonical_json(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => {
            let mut body: Vec<String> = items.iter().map(canonical_json).collect();
            body.sort();
            format!("[{}]", body.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// [`LaunchKitSource::prepare`]'s failure — the manager maps each arm to a different failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    /// After the check, a source of the plan (instructions · skills · preset files) disappeared or changed — the checked plan is already stale.
    /// The manager cuts it off with [`crate::types::PtyError::LaunchRaced`] (it never tried to open the process).
    // TRD S25 §5-2 4 (c)
    Stale(String),
    /// Anything else — the volatile folder could not be made, etc. The manager cuts it off with `SpawnFailed`.
    Failed(String),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(why) => write!(f, "launch-material source changed after the check: {why}"),
            Self::Failed(why) => write!(f, "launch-material preparation failed: {why}"),
        }
    }
}

impl std::error::Error for PrepareError {}

// ── material source port ────────────────────────────────────────────────────────────

/// The source of launch materials — the daemon plugs it in (default = [`NoopLaunchKits`]).
///
/// The manager calls it without holding the roster · reservation locks — the implementation may read the roster inside (asset-library →
/// manager, one direction).
///
/// The four scope methods ([`LaunchKitSource::begin_launch`] · [`LaunchKitSource::check_in`] ·
/// [`LaunchKitSource::prepare_in`] · [`LaunchKitSource::working_folder`]) are overridden all together or not at all,
/// and a wrapper forwards all four.
// ADR-0334
// ADR-0324
pub trait LaunchKitSource: Send + Sync + 'static {
    /// Whether this profile can be launched — when blocked, every reason hit.
    ///
    /// ★Read-only★ — reading the roster and up to [`crate::profile_edit::ProfileEditPort::workspace_lock`].
    /// It never calls profile writes, [`crate::profile_edit::ProfileEditPort::set_attachments`] included: the manager compares the
    /// attachments · command · env before and after this call, so a write here would always break that comparison and end the launch in
    /// a race failure ([`crate::types::PtyError::LaunchRaced`]).
    /// ★The launching agent's own `workspace_lock` reads `Preparing`★ — the launch's spawn reservation is held during
    /// this call, so the check sees its own reservation. Do not read it as an outside lock and block (every launch would
    /// block). A profile not yet in the roster (the pre-create check of an ad-hoc create) also comes here — through
    /// [`LaunchKitSource::check_in`], which the manager calls for every check — and its id's `workspace_lock` is
    /// `NotFound` (not a block reason).
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked>;

    /// Prepares this incarnation's materials per `plan` (volatile folder · merged instructions · skill list). Whatever is made meanwhile is
    /// reclaimed by [`LaunchKitSource::release`] regardless of outcome (the caller arms the reclaim **before** preparing).
    ///
    /// `plan` = the one this launch's check ([`LaunchKitSource::check_in`]) returned — not resolved again from the
    /// profile. If a source vanished or changed after the check, the answer is [`PrepareError::Stale`] (kept apart from
    /// every other failure). The manager reaches this through [`LaunchKitSource::prepare_in`].
    fn prepare(&self, plan: LaunchPlan, id: AgentId, epoch: u32)
        -> Result<LaunchKit, PrepareError>;

    /// Reclaims this incarnation's materials. May be called any number of times, and for an incarnation never prepared.
    ///
    /// Callers = the spawn failure guard (on that spawn thread) · the reaper that reaped a launched incarnation (the global single serial
    /// thread — every later reap queues behind this call). So the implementation:
    /// - is bounded and fast — no waiting · retrying · sleep.
    /// - does not panic — the reaper does not contain it, so in production a panic ends the daemon (ADR-0292).
    /// - reclaims exactly the materials of `(id, epoch)` — it never sweeps the whole id (that id's next incarnation `prepare(id, e2)` can run
    ///   at the same time).
    /// - is best effort — the CLI tree may still be alive holding the volatile folder: it does not wait for handles to close, leaves what it
    ///   cannot delete (the boot sweep backs it), and does not follow reparse points it did not make itself.
    /// - takes no library write lock and never calls back into a manager path that waits for reaping (the reaper would wait on itself).
    // TRD S25 §5-2 4 (c2) ⑤
    fn release(&self, id: AgentId, epoch: u32);

    /// Opens one launch's [`LaunchScope`]. The manager calls it once per launch — after the spawn reservation and the
    /// running check, before any check — and passes the scope to every [`LaunchKitSource::check_in`],
    /// [`LaunchKitSource::prepare_in`] and [`LaunchKitSource::working_folder`] of that launch; it calls `check` /
    /// `prepare` only through those. The scope ends when the launch returns. The default scope is empty.
    // ADR-0360
    fn begin_launch(&self) -> LaunchScope {
        LaunchScope::default()
    }

    /// [`LaunchKitSource::check`] inside one launch. The default ignores the scope and calls `check` — a source whose
    /// `check` delegates here must override this, or the two recurse.
    fn check_in(
        &self,
        _scope: &LaunchScope,
        profile: &AgentProfile,
    ) -> Result<LaunchPlan, LaunchBlocked> {
        self.check(profile)
    }

    /// [`LaunchKitSource::prepare`] inside one launch. The default ignores the scope and calls `prepare` — a source
    /// whose `prepare` delegates here must override this, or the two recurse.
    fn prepare_in(
        &self,
        _scope: &LaunchScope,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        self.prepare(plan, id, epoch)
    }

    /// The working folder the launch spawns in, for the agent's `cwd` — the folder this launch's
    /// [`LaunchKitSource::check_in`] judged for the same `cwd`, so the spawn runs where the check looked. The default
    /// judges nothing: the `dunce` canonical path, or `cwd` as given when that fails — one blocking canonicalize,
    /// unbounded.
    fn working_folder(&self, _scope: &LaunchScope, cwd: &Path) -> PathBuf {
        dunce::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf())
    }
}

/// One launch's shared context, opaque to the manager — a [`LaunchKitSource`] keeps its per-launch state in it
/// ([`LaunchKitSource::begin_launch`]). The default scope holds nothing.
#[derive(Clone, Default)]
pub struct LaunchScope(Option<Arc<dyn Any + Send + Sync>>);

impl LaunchScope {
    pub fn new<T: Any + Send + Sync>(value: Arc<T>) -> Self {
        Self(Some(value))
    }

    /// The value the scope holds when it is a `T` — `None` for an empty scope or a value of another type.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0.clone()?.downcast::<T>().ok()
    }
}

/// No materials — backend = the command variant, no instructions, no settings (ADR-0370). With this plan and kit the
/// backend emits the argv byte for byte as the golden (`tests/launch_argv_golden.rs`). The production daemon plugs the
/// library instead; this stays the manager default (tests, harness bins) and the library's fallback for a backend
/// without a settings check (ADR-0328).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopLaunchKits;

impl LaunchKitSource for NoopLaunchKits {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        Ok(LaunchPlan {
            backend: backend_word(&profile.command),
            instructions: Vec::new(),
            skills: Vec::new(),
            raw_settings: Vec::new(),
            // Not read: `real` is false (the field doc).
            include_priming: true,
            notices: Vec::new(),
            real: false,
            folders: Vec::new(),
            extra_skill_roots: Vec::new(),
            stamp: None,
        })
    }

    fn prepare(
        &self,
        plan: LaunchPlan,
        _id: AgentId,
        _epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        Ok(LaunchKit {
            plan,
            volatile_dir: None,
            instructions_file: None,
            skills: Vec::new(),
            instructions_hash: None,
            last_applied_instructions_hash: None,
        })
    }

    fn release(&self, _id: AgentId, _epoch: u32) {}
}

/// The only way to fill [`LaunchPlan::backend`] — `command`'s serde `kind` tag in lowercase.
// No backend name is written here (ADR-0004).
pub fn backend_word(command: &AgentCommand) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|v| {
            v.get("kind")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default()
}

// ── placement port ─────────────────────────────────────────────────────────────────

/// The way backend placement writes to disk — only inside the volatile folder (never in the working folder).
///
/// There is no delete on purpose — clearing the volatile folder is [`LaunchKitSource::release`]'s job, and it detaches links first so it
/// does not follow a junction and delete library skills.
///
/// ★The production implementation follows links★ — if an ancestor of the written path (`write_file`'s `path` · `link_dir`'s `link`) is
/// a link, it writes inside that target folder (a library skill). So a backend gets this port only as `place_in_volatile`'s confined port
/// (which blocks it with [`LaunchFs::is_link`]). The memory implementation refuses writes inside links.
// No `'static` bound — `place_in_volatile`'s confined port wrapping a borrowed port is one too.
// ADR-0334
pub trait LaunchFs: Send + Sync {
    /// Writes `bytes` to `path` — overwrites if present, creates missing parent folders.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Makes a directory link (Windows = junction) at `link` pointing to the folder `target` — creates missing parent folders.
    /// `target` not absolute or containing NUL = `InvalidInput`, and no parent folder is made either.
    /// Anything at `link` = `AlreadyExists`.
    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()>;

    /// Whether anything is at that spot — links are not followed (a link whose target is gone counts as present). A failed check is `false`
    /// whatever the cause (access denied included).
    fn exists(&self, path: &Path) -> bool;

    /// Whether that spot is a link — links are not followed. Link = a Windows name-surrogate reparse point (junction · symbolic link ·
    /// mount point) · a symbolic link elsewhere. Other reparse points such as cloud placeholders are not links.
    /// `Ok(false)` = something that is not a link is there · `NotFound` = nothing · any other `Err` = the check failed.
    fn is_link(&self, path: &Path) -> io::Result<bool>;
}

/// Production placement — std file writes + platform directory links.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdLaunchFs;

impl LaunchFs for StdLaunchFs {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        create_parent(path)?;
        std::fs::write(path, bytes)
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        check_link_target(target)?;
        create_parent(link)?;
        engram_dashboard_platform::fs::link_dir(target, link)
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }

    // Windows std `is_symlink` = the name-surrogate tags among reparse points (junction · symbolic link).
    fn is_link(&self, path: &Path) -> io::Result<bool> {
        std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink())
    }
}

// platform `link_dir` checks the same two, but after making the parent folders, so a refused target leaves an empty folder — check before.
fn check_link_target(target: &Path) -> io::Result<()> {
    if !target.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target must be an absolute path",
        ));
    }
    if target.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link target contains NUL",
        ));
    }
    Ok(())
}

fn create_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent),
        None => Ok(()),
    }
}

/// Runs `place` with a placement port confined to this incarnation's volatile folder — `backend::materialize` calls backend placement only
/// through this.
///
/// Blocked writes (`write_file`'s `path` · `link_dir`'s `link`):
/// - Outside the volatile folder · the volatile folder itself. Checked in the original spelling before normalizing — the head may differ
///   from the volatile folder's spelling only in case ([`engram_dashboard_platform::fs::same_path`] — Windows folds), and a different
///   separator · prefix (`\\?\` etc.) spelling is outside.
/// - Among the words after the volatile folder split on both `\` and `/`: an empty one (doubled separator) · `.` · `..` · one ending in
///   `.` or a space · one containing `:` — blocked on every OS.
///   Windows opens a name with its trailing dots · spaces stripped, landing on the same spot as another name, and `:` is an alternate data stream.
/// - Any existing link ([`LaunchFs::is_link`] — asks the disk) from right under the volatile folder up to the written spot's parent
///   (for `write_file`, up to the spot itself). Both links the preparation made and links this placement made; a case-changed spelling
///   is caught because the disk answers for the same spot (8.3 name spellings too — measured: blocked, S25 P4b-2 re-review 2026-11-01).
///   Blocked too when that check fails with an error other than `NotFound`.
///
/// Not guaranteed: a race where someone inserts a link between the check and the write — placement runs before the CLI is launched and
/// the volatile folder belongs to this incarnation, so nobody writes there meanwhile. The volatile folder itself and above are not
/// checked (the preparation made them). Reparse points that are not name surrogates do not count as links.
///
/// A blocked write = a warning log + `PermissionDenied`, and if there was any, the result is `Err` whatever `place` answered (even if the
/// backend swallows the error the placement fails · if `place` was `Err` too, its reason is appended). With no volatile folder every
/// write is blocked. `exists` · `is_link` pass through. A volatile folder that is not absolute = `Err` without calling `place` — a
/// relative path would resolve against the daemon's current folder.
// ADR-0334
// TRD S25 §1-5 (na) 6
pub(crate) fn place_in_volatile<T>(
    kit: &LaunchKit,
    fs: &dyn LaunchFs,
    place: impl FnOnce(&dyn LaunchFs) -> Result<T, String>,
) -> Result<T, String> {
    let root = kit.volatile_dir.as_deref();
    if let Some(root) = root.filter(|r| !r.is_absolute()) {
        return Err(format!(
            "volatile folder is not an absolute path: {}",
            root.display()
        ));
    }
    let confined = VolatileOnly {
        inner: fs,
        root,
        refused: Mutex::new(None),
    };
    let placed = place(&confined);
    if let Some((path, why)) = sync::lock(&confined.refused).take() {
        let refused = format!(
            "blocked a volatile-folder placement write ({why}): {}",
            path.display()
        );
        return Err(match placed {
            Ok(_) => refused,
            Err(e) => format!("{refused}  · placement error: {e}"),
        });
    }
    placed
}

/// [`place_in_volatile`]'s confined port.
struct VolatileOnly<'a> {
    inner: &'a dyn LaunchFs,
    root: Option<&'a Path>,
    /// The first blocked path and why.
    refused: Mutex<Option<(PathBuf, &'static str)>>,
}

impl VolatileOnly<'_> {
    /// `through_self` = block even when the written spot itself is a link (file writes).
    fn admit(&self, path: &Path, through_self: bool) -> io::Result<()> {
        let Some(why) = self.refusal(path, through_self) else {
            return Ok(());
        };
        tracing::warn!(path = %path.display(), why, "blocked a volatile-folder placement write");
        sync::lock(&self.refused).get_or_insert_with(|| (path.to_path_buf(), why));
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("blocked a volatile-folder placement write: {why}"),
        ))
    }

    fn refusal(&self, path: &Path, through_self: bool) -> Option<&'static str> {
        let Some(root) = self.root else {
            return Some("no volatile folder");
        };
        if let Some(why) = spelling_refusal(path, root) {
            return Some(why);
        }
        let mut parts = path.components();
        let head: PathBuf = parts.by_ref().take(root.components().count()).collect();
        if !same_path(&head, &root.components().collect::<PathBuf>()) {
            return Some("outside the volatile folder");
        }
        let rest: Vec<Component> = parts.collect();
        if rest.is_empty() {
            return Some("the volatile folder itself");
        }
        if !rest.iter().all(plain_name) {
            return Some("a name with `..` · a trailing dot · space · `:`");
        }
        let walked = if through_self {
            rest.len()
        } else {
            rest.len() - 1
        };
        let mut at = head;
        for name in &rest[..walked] {
            at.push(name);
            match self.inner.is_link(&at) {
                Ok(false) => {}
                Ok(true) => return Some("past a link"),
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(_) => return Some("could not check for a link"),
            }
        }
        None
    }
}

/// Checks the original spelling first — `components()` normalizes separators and drops `.` · doubled separators, so checking after it
/// would miss them. The head may differ from the volatile folder's spelling only in case (a different separator · prefix spelling is
/// outside), and the rest is split on both `\` and `/` and checked word by word. A non-UTF-8 path is blocked.
fn spelling_refusal(path: &Path, root: &Path) -> Option<&'static str> {
    let (Some(text), Some(root_text)) = (path.to_str(), root.to_str()) else {
        return Some("a non-UTF-8 path");
    };
    let is_sep = |b: u8| b == b'\\' || b == b'/';
    let Some(head) = text.get(..root_text.len()) else {
        return Some("outside the volatile folder");
    };
    let separators_match = head
        .bytes()
        .zip(root_text.bytes())
        .all(|(a, b)| a == b || !(is_sep(a) || is_sep(b)));
    if !separators_match || !same_path(Path::new(head), root) {
        return Some("outside the volatile folder");
    }
    let mut rest = &text[root_text.len()..];
    if !root_text.bytes().last().is_some_and(is_sep) {
        match rest.bytes().next() {
            None => return Some("the volatile folder itself"),
            Some(b) if is_sep(b) => rest = &rest[1..],
            Some(_) => return Some("outside the volatile folder"),
        }
    }
    if rest.is_empty() {
        return Some("the volatile folder itself");
    }
    let bad = |name: &str| {
        name.is_empty()
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
            || name.contains(':')
    };
    if rest.split(['\\', '/']).any(bad) {
        return Some("an empty word · `.` · `..` · a name with a trailing dot · space · `:`");
    }
    None
}

fn plain_name(component: &Component) -> bool {
    let Component::Normal(name) = component else {
        return false;
    };
    let bytes = name.as_encoded_bytes();
    !matches!(bytes.last(), Some(b'.' | b' ')) && !bytes.contains(&b':')
}

impl LaunchFs for VolatileOnly<'_> {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.admit(path, true)?;
        self.inner.write_file(path, bytes)
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        self.admit(link, false)?;
        self.inner.link_dir(link, target)
    }

    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }

    fn is_link(&self, path: &Path) -> io::Result<bool> {
        self.inner.is_link(path)
    }
}

/// Where a backend placement links the kit's skills: `<volatile>/<SKILL_LINKS_PARENT>/<SKILL_LINKS_DIR>/<name>`, one
/// directory link (Windows: a junction) per skill. claude passes `<volatile>/<SKILL_LINKS_PARENT>` as its plugin
/// folder; codex passes `<volatile>/<SKILL_LINKS_PARENT>/<SKILL_LINKS_DIR>` as one extra skill root. One definition,
/// so the links of both backends sit at the same depth. A release outside this crate cannot see these constants;
/// it must detach every directory link under the volatile folder whatever its depth.
// ADR-0316
// ADR-0337
pub(crate) const SKILL_LINKS_PARENT: &str = "engram";
/// See [`SKILL_LINKS_PARENT`].
pub(crate) const SKILL_LINKS_DIR: &str = "skills";

/// The result of backend placement — the paths and argument pieces that backend puts on its command line · channel. Default = nothing to carry.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct LaunchArtifacts {
    /// [`LaunchPlan::real`] as is.
    pub real: bool,
    pub args: Vec<String>,
    pub instructions_file: Option<PathBuf>,
    /// The skill roots. claude: the plugin folder (inside the volatile folder). codex JSON: the extra skill roots in
    /// order — the junction root inside the volatile folder (only with ≥ 1 skill), then the plan's
    /// [`LaunchPlan::extra_skill_roots`] (outside it).
    pub skill_roots: Vec<PathBuf>,
    /// The merged backend settings file (inside the volatile folder). `None` = no raw settings to load.
    // ADR-0328
    pub settings_file: Option<PathBuf>,
    /// Folders the CLI is given access to, in [`LaunchPlan::folders`] order.
    // ADR-0341
    pub add_dirs: Vec<PathBuf>,
    /// Environment pairs the backend adds to the process (claude's `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD`),
    /// meant to be applied after the profile env so they override a profile value for the same key.
    // ADR-0341
    pub env: Vec<(String, String)>,
    /// codex: the arguments the preset raw settings add (`-c`, `key=value`, …), in plan order, exactly as placed on
    /// the line — after the pass-through, before the MCP attachment. Both modes.
    // ADR-0337
    pub config_args: Vec<String>,
    /// codex terminal: the `developer_instructions='…'` value built from the merged instructions file, placed last.
    /// `None` = no priming on the line.
    // ADR-0337
    pub instructions_override: Option<String>,
    /// codex JSON: the merged instructions text (read from [`LaunchKit::instructions_file`]). `None` = none or blank.
    /// Sent as `developerInstructions` on `thread/start`, and on a `thread/resume` when the transport's version gate
    /// lets it through (`RESUME_INSTRUCTIONS_FLOOR` in `backend/codex/transport.rs`).
    // ADR-0337
    pub instructions_text: Option<String>,
    /// codex JSON: there is instructions text ([`LaunchArtifacts::instructions_text`] is `Some`), and the kit's
    /// instructions hash is set and differs from [`LaunchKit::last_applied_instructions_hash`] — a resume then tells the
    /// conversation the instructions changed. Never `true` without text.
    // ADR-0337
    pub instructions_changed: bool,
}

// ── guard · bundle ───────────────────────────────────────────────────────────────

/// A guard that reclaims this incarnation's materials wherever the spawn fails before it ends — dropped while armed, it calls
/// [`LaunchKitSource::release`] exactly once. On spawn success, [`LaunchKitGuard::disarm`].
///
/// Independent of the control channel's `ProvisionGuard`. `release` deletes from disk, so never drop it while holding a lock.
// ADR-0334
pub(crate) struct LaunchKitGuard {
    kits: Arc<dyn LaunchKitSource>,
    id: AgentId,
    epoch: u32,
    armed: bool,
}

impl LaunchKitGuard {
    /// Arms **before** [`LaunchKitSource::prepare`] — a failure during preparation is reclaimed too.
    pub(crate) fn arm(kits: Arc<dyn LaunchKitSource>, id: AgentId, epoch: u32) -> Self {
        Self {
            kits,
            id,
            epoch,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    /// This incarnation's reclaim lease to put on the session — reclaims through the very source the guard holds (the one that prepared).
    pub(crate) fn lease(&self) -> LaunchLease {
        LaunchLease {
            kits: self.kits.clone(),
            id: self.id,
            epoch: self.epoch,
        }
    }
}

impl Drop for LaunchKitGuard {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                "launch-material guard fired; failure before session registration, reclaiming materials"
            );
            self.kits.release(self.id, self.epoch);
        }
    }
}

/// The lease for reclaiming a launched incarnation's materials — the session holds it, and the reaper that removed that session from the roster takes it out and uses it once.
///
/// The type keeps it single-use — no `Clone`, [`LaunchLease::release`] consumes the lease, and the session hands it out only by taking
/// (`AgentSession::take_launch_lease`).
/// ★Do not add `Drop`★: observers · harnesses hold the session `Arc` too, so where it drops last is unknown — reclaiming on drop would
///   make two reclaimers. After session registration the only reclaimer is the reaper filtered by the incarnation marker.
/// The source comes from [`LaunchKitGuard::lease`] — the port that prepared that incarnation, not the port caught at manager construction.
// ADR-0334
// TRD S25 §5-2 4 (c2) ② · ③
pub(crate) struct LaunchLease {
    kits: Arc<dyn LaunchKitSource>,
    id: AgentId,
    epoch: u32,
}

impl LaunchLease {
    #[cfg(test)]
    pub(crate) fn new(kits: Arc<dyn LaunchKitSource>, id: AgentId, epoch: u32) -> Self {
        Self { kits, id, epoch }
    }

    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Reclaims — called while holding no lock (`release` deletes from disk).
    pub(crate) fn release(self) {
        self.kits.release(self.id, self.epoch);
    }
}

/// The two launch-material ports the manager takes.
#[derive(Clone)]
pub struct LaunchPorts {
    pub kits: Arc<dyn LaunchKitSource>,
    pub fs: Arc<dyn LaunchFs>,
}

impl LaunchPorts {
    /// No materials ([`NoopLaunchKits`]) + the production placement.
    pub fn noop() -> Self {
        Self {
            kits: Arc::new(NoopLaunchKits),
            fs: Arc::new(StdLaunchFs),
        }
    }
}

// ── in-memory placement for tests ─────────────────────────────────────────────────────────

/// A diskless [`LaunchFs`] — folders exist only as ancestors of entries, and nothing is written inside links.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemLaunchFs {
    entries: std::sync::Mutex<std::collections::BTreeMap<PathBuf, MemEntry>>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MemEntry {
    File(Vec<u8>),
    Link(PathBuf),
}

#[cfg(test)]
impl MemLaunchFs {
    /// Every spot written (files · links — parent folders made implicitly are not included), in path order.
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        engram_dashboard_base::sync::lock(&self.entries)
            .keys()
            .cloned()
            .collect()
    }

    pub(crate) fn file(&self, path: &Path) -> Option<Vec<u8>> {
        match engram_dashboard_base::sync::lock(&self.entries).get(path) {
            Some(MemEntry::File(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    pub(crate) fn link_target(&self, path: &Path) -> Option<PathBuf> {
        match engram_dashboard_base::sync::lock(&self.entries).get(path) {
            Some(MemEntry::Link(target)) => Some(target.clone()),
            _ => None,
        }
    }

    fn occupied(entries: &std::collections::BTreeMap<PathBuf, MemEntry>, path: &Path) -> bool {
        entries.contains_key(path) || entries.keys().any(|k| k != path && k.starts_with(path))
    }

    fn parent_is_not_a_dir(
        entries: &std::collections::BTreeMap<PathBuf, MemEntry>,
        path: &Path,
    ) -> bool {
        path.ancestors().skip(1).any(|a| entries.contains_key(a))
    }
}

#[cfg(test)]
impl LaunchFs for MemLaunchFs {
    fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut entries = engram_dashboard_base::sync::lock(&self.entries);
        if Self::parent_is_not_a_dir(&entries, path) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "parent is not a folder",
            ));
        }
        let is_dir = entries.keys().any(|k| k != path && k.starts_with(path));
        if is_dir || matches!(entries.get(path), Some(MemEntry::Link(_))) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "the spot is a folder · link",
            ));
        }
        entries.insert(path.to_path_buf(), MemEntry::File(bytes.to_vec()));
        Ok(())
    }

    fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
        check_link_target(target)?;
        let mut entries = engram_dashboard_base::sync::lock(&self.entries);
        if Self::parent_is_not_a_dir(&entries, link) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "parent is not a folder",
            ));
        }
        if Self::occupied(&entries, link) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "something already exists at the link path",
            ));
        }
        entries.insert(link.to_path_buf(), MemEntry::Link(target.to_path_buf()));
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        Self::occupied(&engram_dashboard_base::sync::lock(&self.entries), path)
    }

    fn is_link(&self, path: &Path) -> io::Result<bool> {
        let entries = engram_dashboard_base::sync::lock(&self.entries);
        match entries.get(path) {
            Some(MemEntry::Link(_)) => Ok(true),
            Some(MemEntry::File(_)) => Ok(false),
            None if Self::occupied(&entries, path) => Ok(false),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use serde_json::json;
    use uuid::Uuid;

    use crate::profile::AgentOutputFormat;

    fn profile(command: AgentCommand) -> AgentProfile {
        AgentProfile::new(
            "golden".to_string(),
            command,
            PathBuf::from("work"),
            Vec::new(),
            false,
        )
    }

    fn claude() -> AgentCommand {
        AgentCommand::Claude {
            extra_args: vec!["--x".to_string()],
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn reason(code: &str, args: Value, line: Option<u32>) -> Reason {
        Reason {
            code: code.to_string(),
            args,
            line,
        }
    }

    /// Counts the `release` calls.
    #[derive(Default)]
    struct CountingKits {
        released: Mutex<Vec<(AgentId, u32)>>,
    }

    impl LaunchKitSource for CountingKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            NoopLaunchKits.check(profile)
        }
        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            NoopLaunchKits.prepare(plan, id, epoch)
        }
        fn release(&self, id: AgentId, epoch: u32) {
            self.released.lock().unwrap().push((id, epoch));
        }
    }

    // ── Noop ──

    #[test]
    fn noop_check_plans_no_real_kit_and_names_the_command_variant() {
        let codex = AgentCommand::Codex {
            extra_args: Vec::new(),
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "sh".to_string(),
            args: Vec::new(),
        };
        for (command, word) in [(claude(), "claude"), (codex, "codex"), (shell, "shell")] {
            let plan = NoopLaunchKits
                .check(&profile(command))
                .expect("Noop never blocks");
            assert_eq!(
                plan,
                LaunchPlan {
                    backend: word.to_string(),
                    instructions: Vec::new(),
                    skills: Vec::new(),
                    raw_settings: Vec::new(),
                    include_priming: true,
                    notices: Vec::new(),
                    real: false,
                    folders: Vec::new(),
                    extra_skill_roots: Vec::new(),
                    stamp: None,
                }
            );
        }
    }

    #[test]
    fn noop_prepare_makes_a_kit_without_a_folder_and_release_does_nothing() {
        let p = profile(claude());
        let plan = NoopLaunchKits.check(&p).unwrap();
        let kit = NoopLaunchKits.prepare(plan.clone(), p.id, 3).unwrap();
        assert_eq!(
            kit,
            LaunchKit {
                plan,
                volatile_dir: None,
                instructions_file: None,
                skills: Vec::new(),
                instructions_hash: None,
                last_applied_instructions_hash: None,
            }
        );
        NoopLaunchKits.release(p.id, 3);
        NoopLaunchKits.release(p.id, 3);
    }

    #[test]
    fn noop_ports_plan_no_real_kit() {
        let ports = LaunchPorts::noop();
        assert!(!ports.kits.check(&profile(claude())).unwrap().real);
    }

    // ── guard ──

    #[test]
    fn guard_releases_exactly_once_when_dropped_armed() {
        let kits = Arc::new(CountingKits::default());
        let id = Uuid::from_u128(7);
        drop(LaunchKitGuard::arm(kits.clone(), id, 4));
        assert_eq!(*kits.released.lock().unwrap(), vec![(id, 4)]);
    }

    #[test]
    fn guard_never_releases_after_disarm() {
        let kits = Arc::new(CountingKits::default());
        let mut guard = LaunchKitGuard::arm(kits.clone(), Uuid::from_u128(7), 4);
        guard.disarm();
        drop(guard);
        assert!(kits.released.lock().unwrap().is_empty());
    }

    #[test]
    fn guard_releases_on_an_early_return() {
        fn fails(kits: Arc<dyn LaunchKitSource>) -> Result<(), String> {
            let mut guard = LaunchKitGuard::arm(kits, Uuid::from_u128(9), 1);
            Err::<(), _>("prepare failed".to_string())?;
            guard.disarm();
            Ok(())
        }
        let kits = Arc::new(CountingKits::default());
        assert!(fails(kits.clone()).is_err());
        assert_eq!(
            *kits.released.lock().unwrap(),
            vec![(Uuid::from_u128(9), 1)]
        );
    }

    #[test]
    fn ports_and_guard_cross_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<LaunchPorts>();
        send_sync::<LaunchKitGuard>();
    }

    // ── blocks ──

    #[test]
    fn block_reasons_sort_in_the_declared_order() {
        let r = || reason("x", Value::Null, None);
        let mut reasons = vec![
            BlockReason::LibraryUnavailable { reason: r() },
            BlockReason::Conflict {
                reason: r(),
                items: Vec::new(),
            },
            BlockReason::Broken {
                item: "a".to_string(),
                via: Vec::new(),
                reason: r(),
            },
            BlockReason::Missing {
                item: "z".to_string(),
                via: Vec::new(),
                reason: r(),
            },
        ];
        reasons.sort();
        let order: Vec<&str> = reasons
            .iter()
            .map(|b| match b {
                BlockReason::Missing { .. } => "missing",
                BlockReason::Broken { .. } => "broken",
                BlockReason::Conflict { .. } => "conflict",
                BlockReason::LibraryUnavailable { .. } => "library",
            })
            .collect();
        assert_eq!(order, ["missing", "broken", "conflict", "library"]);
    }

    #[test]
    fn reason_order_agrees_with_equality() {
        let samples = [
            json!(null),
            json!(false),
            json!(true),
            json!(-3),
            json!(0),
            json!(7),
            json!(u64::MAX),
            json!(0.0),
            json!(-0.0),
            json!(2.5),
            json!("a"),
            json!("b"),
            json!([]),
            json!([1, 2]),
            json!([1, 3]),
            json!({}),
            json!({"a": 1, "b": 2}),
            json!({"b": 2, "a": 1}),
            json!({"a": 1, "c": 0}),
        ];
        let reasons: Vec<Reason> = samples
            .iter()
            .flat_map(|v| {
                [
                    reason("c", v.clone(), None),
                    reason("c", v.clone(), Some(2)),
                ]
            })
            .collect();
        for a in &reasons {
            for b in &reasons {
                assert_eq!(a.cmp(b) == Ordering::Equal, a == b, "{a:?} vs {b:?}");
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a:?} vs {b:?}");
                for c in &reasons {
                    if a.cmp(b) != Ordering::Greater && b.cmp(c) != Ordering::Greater {
                        assert_ne!(a.cmp(c), Ordering::Greater, "{a:?} ≤ {b:?} ≤ {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn blocked_display_names_each_reason() {
        let blocked = LaunchBlocked {
            reasons: vec![
                BlockReason::Missing {
                    item: "team/a.yaml".to_string(),
                    via: vec!["team/base.yaml".to_string()],
                    reason: reason("missing-file", Value::Null, None),
                },
                BlockReason::Conflict {
                    reason: reason("skill-name", json!({"name": "s"}), None),
                    items: vec![
                        ("x/s".to_string(), Vec::new()),
                        ("y/s".to_string(), Vec::new()),
                    ],
                },
                BlockReason::LibraryUnavailable {
                    reason: reason("root-unset", Value::Null, None),
                },
            ],
        };
        assert_eq!(
            blocked.to_string(),
            "launch blocked: not found team/a.yaml (missing-file) · conflict x/s, y/s (skill-name) · library folder missing (root-unset)"
        );
    }

    // An item can be a hand-edited spelling from the profile file; the manager logs this line.
    #[test]
    fn blocked_display_escapes_control_characters_in_items() {
        let blocked = LaunchBlocked {
            reasons: vec![
                BlockReason::Missing {
                    item: "a\nb.yaml".to_string(),
                    via: Vec::new(),
                    reason: reason("not-found", Value::Null, None),
                },
                BlockReason::Broken {
                    item: "../x\u{1b}[31m".to_string(),
                    via: Vec::new(),
                    reason: reason("item-path-invalid", Value::Null, None),
                },
                BlockReason::Conflict {
                    reason: reason("skill-name", json!({"name": "s"}), None),
                    items: vec![("x\r/s".to_string(), Vec::new())],
                },
            ],
        };
        let line = blocked.to_string();
        assert_eq!(
            line,
            "launch blocked: not found a\\nb.yaml (not-found) · error ../x\\u{1b}[31m (item-path-invalid) · conflict x\\r/s (skill-name)"
        );
        assert!(!line.chars().any(char::is_control), "{line:?}");
    }

    // ── placement ──

    fn mem_root() -> PathBuf {
        std::env::temp_dir().join("engram-mem-launch-fs")
    }

    #[test]
    fn mem_fs_writes_with_implicit_parents_and_overwrites() {
        let fs = MemLaunchFs::default();
        let file = mem_root().join("v").join("engram").join("settings.json");
        fs.write_file(&file, b"one").unwrap();
        fs.write_file(&file, b"two").unwrap();
        assert_eq!(fs.file(&file).as_deref(), Some(&b"two"[..]));
        assert!(fs.exists(&file));
        assert!(fs.exists(&mem_root().join("v")));
        assert!(!fs.exists(&mem_root().join("w")));
    }

    #[test]
    fn mem_fs_links_once_and_wants_an_absolute_target() {
        let fs = MemLaunchFs::default();
        let link = mem_root().join("v").join("skills").join("s");
        let target = mem_root().join("library").join("s");
        for bad in [PathBuf::from("relative"), mem_root().join("tar\0get")] {
            assert_eq!(
                fs.link_dir(&link, &bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert!(fs.paths().is_empty());
        fs.link_dir(&link, &target).unwrap();
        assert_eq!(fs.link_target(&link), Some(target.clone()));
        assert!(fs.exists(&link));
        assert_eq!(
            fs.link_dir(&link, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs.link_dir(&mem_root().join("v"), &target)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(fs.write_file(&link, b"x").is_err());
        assert!(fs.write_file(&link.join("through"), b"x").is_err());
    }

    #[test]
    fn mem_fs_paths_are_the_written_files_and_links_only() {
        let fs = MemLaunchFs::default();
        let volatile = mem_root().join("v");
        let file = volatile.join("engram").join("settings.json");
        let link = volatile.join("skills").join("s");
        fs.write_file(&file, b"x").unwrap();
        fs.link_dir(&link, &mem_root().join("library").join("s"))
            .unwrap();
        assert_eq!(fs.paths(), vec![file, link]);
    }

    // ── volatile folder confinement ──

    fn kit_in(volatile_dir: Option<PathBuf>) -> LaunchKit {
        let p = profile(claude());
        let mut kit = NoopLaunchKits
            .prepare(NoopLaunchKits.check(&p).unwrap(), p.id, 1)
            .unwrap();
        kit.volatile_dir = volatile_dir;
        kit
    }

    /// ★Placement writes reach the given port only inside the volatile folder★ — outside · the folder itself · a path escaping by `..` · a name
    /// ending in a dot · space or containing `:` · past a link this placement made do not reach the port, and if there was any, the whole
    /// placement fails (even if the backend swallows the error).
    // TRD S25 §1-5 (na) 6
    #[test]
    fn placement_writes_reach_the_fs_only_inside_the_volatile_folder() {
        let volatile = mem_root().join("launch").join("a-1");
        let library = mem_root().join("library").join("s");
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            fs.write_file(&volatile.join("engram").join("settings.json"), b"{}")
                .map_err(|e| e.to_string())?;
            fs.link_dir(&volatile.join("engram").join("skills").join("s"), &library)
                .map_err(|e| e.to_string())?;
            assert!(fs.exists(&volatile.join("engram")), "checks pass through");
            Ok("placed")
        });
        assert_eq!(placed, Ok("placed"));
        assert_eq!(
            fs.paths(),
            vec![
                volatile.join("engram").join("settings.json"),
                volatile.join("engram").join("skills").join("s"),
            ]
        );

        let work = mem_root().join("work");
        // A path built in the original spelling — `join` cannot build this shape.
        let main = std::path::MAIN_SEPARATOR;
        let alt = if main == '\\' { '/' } else { '\\' };
        let root_text = volatile.to_string_lossy().into_owned();
        let last_sep = root_text.rfind(main).unwrap();
        let mut mixed_root = root_text.clone();
        mixed_root.replace_range(last_sep..last_sep + 1, &alt.to_string());
        let spelled = |s: String| PathBuf::from(s);
        let outside: Vec<(&str, PathBuf)> = vec![
            (
                "uppercase · other-separator head + `.`",
                spelled(format!(
                    "{}{alt}.{alt}sub\\x",
                    root_text.to_uppercase().replace(main, &alt.to_string())
                )),
            ),
            (
                "mixed separators in the head",
                spelled(format!("{mixed_root}{main}engram{main}x")),
            ),
            (
                "doubled separator",
                spelled(format!("{root_text}{main}{main}engram{main}x")),
            ),
            (
                "`.` word",
                spelled(format!("{root_text}{main}.{main}engram{main}x")),
            ),
            ("working folder", work.join("AGENTS.override.md")),
            ("the volatile folder itself", volatile.clone()),
            (
                "neighbouring incarnation",
                mem_root().join("launch").join("a-10").join("x"),
            ),
            ("escapes by `..`", volatile.join("..").join("a-2").join("x")),
            (
                "past a link",
                volatile
                    .join("engram")
                    .join("skills")
                    .join("s")
                    .join("SKILL.md"),
            ),
            (
                "the link spot itself",
                volatile.join("engram").join("skills").join("s"),
            ),
            ("trailing dot", volatile.join("engram.").join("x")),
            ("trailing space", volatile.join("engram ").join("x")),
            ("stream", volatile.join("engram").join("x:y")),
            ("dots-only name", volatile.join("...").join("x")),
        ];
        for (what, path) in outside {
            let fs = MemLaunchFs::default();
            let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
                fs.link_dir(&volatile.join("engram").join("skills").join("s"), &library)
                    .map_err(|e| e.to_string())?;
                let refused = fs.write_file(&path, b"x").unwrap_err();
                assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{what}");
                Ok(())
            });
            assert!(
                matches!(&placed, Err(m) if m.contains(&path.display().to_string())),
                "{what}: {placed:?}"
            );
            assert_eq!(
                fs.paths(),
                vec![volatile.join("engram").join("skills").join("s")],
                "{what}: a blocked write reached the port"
            );
        }

        // Links too — a blocked link spot does not reach the port.
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            let _ = fs.link_dir(&work.join(".agents").join("skills").join("s"), &library);
            Ok(())
        });
        assert!(
            placed.is_err(),
            "a swallowed error still fails the placement"
        );
        assert!(fs.paths().is_empty());

        // If the backend failed for its own reason, that reason is appended after the blocked writes (not replaced).
        let placed: Result<(), String> =
            place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
                let _ = fs.write_file(&work.join("x"), b"x");
                Err("backend reason".to_string())
            });
        let message = placed.unwrap_err();
        assert!(
            message.contains(&work.join("x").display().to_string())
                && message.contains("backend reason"),
            "{message}"
        );
    }

    /// The head is compared with case folded as the OS does — on a folding OS (Windows) a volatile-folder spelling differing only in case is
    /// inside; on a non-folding OS it is outside.
    #[test]
    fn placement_compares_the_volatile_prefix_as_the_os_does() {
        let volatile = mem_root().join("launch").join("a-1");
        let spelled = PathBuf::from(volatile.to_string_lossy().to_uppercase())
            .join("engram")
            .join("x");
        let folds = same_path(Path::new("a"), Path::new("A"));
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(Some(volatile)), &fs, |fs| {
            fs.write_file(&spelled, b"x").map_err(|e| e.to_string())
        });
        assert_eq!(placed.is_ok(), folds, "{placed:?}");
    }

    /// If whether an ancestor is a link cannot be checked (an error other than `NotFound`), the write is blocked.
    #[test]
    fn placement_refuses_when_a_link_check_fails() {
        struct Unsure(MemLaunchFs);
        impl LaunchFs for Unsure {
            fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
                self.0.write_file(path, bytes)
            }
            fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
                self.0.link_dir(link, target)
            }
            fn exists(&self, path: &Path) -> bool {
                self.0.exists(path)
            }
            fn is_link(&self, _path: &Path) -> io::Result<bool> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        }
        let volatile = mem_root().join("launch").join("a-1");
        let fs = Unsure(MemLaunchFs::default());
        let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &fs, |fs| {
            fs.write_file(&volatile.join("engram").join("x"), b"x")
                .map_err(|e| e.to_string())
        });
        assert!(placed.is_err(), "{placed:?}");
        assert!(fs.0.paths().is_empty());
    }

    /// With no volatile folder every write is blocked, and with a relative path placement is not even called.
    #[test]
    fn placement_without_an_absolute_volatile_folder_writes_nothing() {
        let fs = MemLaunchFs::default();
        let placed = place_in_volatile(&kit_in(None), &fs, |fs| {
            let _ = fs.write_file(&mem_root().join("x"), b"x");
            Ok(())
        });
        assert!(placed.is_err());
        assert_eq!(
            place_in_volatile(&kit_in(None), &fs, |_| Ok(7)),
            Ok(7),
            "a placement that writes nothing succeeds"
        );
        assert!(fs.paths().is_empty());

        let called = std::cell::Cell::new(false);
        let placed = place_in_volatile(
            &kit_in(Some(PathBuf::from("relative").join("launch"))),
            &fs,
            |_| {
                called.set(true);
                Ok(())
            },
        );
        assert!(
            matches!(&placed, Err(m) if m.contains("absolute path")),
            "{placed:?}"
        );
        assert!(
            !called.get(),
            "placement was called with a relative volatile folder"
        );
    }

    /// A temp folder deleted on drop — the links inside are detached first.
    struct Scratch {
        root: PathBuf,
        links: Vec<PathBuf>,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for link in &self.links {
                let _ = engram_dashboard_platform::fs::unlink_dir(link);
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn std_fs_writes_and_links_with_missing_parents() {
        let mut scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let fs = StdLaunchFs;
        let target = scratch.root.join("library").join("skill");
        fs.write_file(&target.join("SKILL.md"), b"body").unwrap();
        assert!(fs.exists(&target.join("SKILL.md")));

        let link = scratch.root.join("volatile").join("skills").join("skill");
        scratch.links.push(link.clone());
        fs.link_dir(&link, &target).unwrap();
        assert_eq!(std::fs::read(link.join("SKILL.md")).unwrap(), b"body");
        assert_eq!(
            fs.link_dir(&link, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );

        let dangling = scratch.root.join("volatile").join("skills").join("gone");
        scratch.links.push(dangling.clone());
        fs.link_dir(&dangling, &scratch.root.join("nowhere"))
            .unwrap();
        assert!(fs.exists(&dangling));
        assert!(!fs.exists(&scratch.root.join("nowhere")));
    }

    #[test]
    fn std_fs_rejects_a_bad_link_target_before_making_parents() {
        let scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let link = scratch.root.join("volatile").join("skills").join("skill");
        for bad in [PathBuf::from("relative"), scratch.root.join("tar\0get")] {
            assert_eq!(
                StdLaunchFs.link_dir(&link, &bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
        assert!(
            !scratch.root.exists(),
            "a refused target left the parent folder"
        );
    }

    /// ★On a real disk, a changed spelling still never writes through a link★ — writing around a link the preparation made in the volatile
    /// folder (Windows = junction · elsewhere = symbolic link) by case · trailing dot · trailing space · stream name, or stacking links, leaves
    /// the library skill folder untouched. A case-only spelling is the same spot only when the volume folds case, so on a non-folding volume
    /// it goes into a new folder inside the volatile folder — that branch is decided by asking the disk (not by OS).
    // TRD S25 §1-5 (na) 6
    #[test]
    fn placement_never_writes_through_a_link_on_a_real_disk() {
        let mut scratch = Scratch {
            root: std::env::temp_dir().join(format!("engram-launch-fs-{}", Uuid::new_v4())),
            links: Vec::new(),
        };
        let library = scratch.root.join("library").join("skills");
        let skill = library.join("s").join("SKILL.md");
        StdLaunchFs.write_file(&skill, b"body").unwrap();
        let volatile = scratch.root.join("launch").join("a-1");
        std::fs::create_dir_all(&volatile).unwrap();
        let link = volatile.join("skills");
        scratch.links.push(link.clone());
        engram_dashboard_platform::fs::link_dir(&library, &link).unwrap();
        let folds = StdLaunchFs.exists(&volatile.join("SKILLS"));
        let elsewhere = scratch.root.join("elsewhere");

        let library_untouched = |what: &str| {
            assert_eq!(std::fs::read(&skill).unwrap(), b"body", "{what}");
            let mut names: Vec<String> = std::fs::read_dir(&library)
                .unwrap()
                .chain(std::fs::read_dir(library.join("s")).unwrap())
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(names, ["SKILL.md", "s"], "{what}: new entry in the library");
        };

        enum Write {
            File(PathBuf),
            Link(PathBuf),
        }
        let attempts = [
            ("as is", Write::File(link.join("s").join("x.md")), true),
            (
                "overwrite",
                Write::File(link.join("s").join("SKILL.md")),
                true,
            ),
            ("the link spot itself", Write::File(link.clone()), true),
            (
                "case",
                Write::File(volatile.join("SKILLS").join("s").join("x.md")),
                folds,
            ),
            (
                "trailing dot",
                Write::File(volatile.join("skills.").join("s").join("x.md")),
                true,
            ),
            (
                "trailing space",
                Write::File(volatile.join("skills ").join("s").join("x.md")),
                true,
            ),
            (
                "stream",
                Write::File(link.join("s").join("SKILL.md:x")),
                true,
            ),
            (
                "link inside a link",
                Write::Link(volatile.join("SKILLS").join("t")),
                folds,
            ),
        ];
        for (what, write, refused) in attempts {
            let placed = place_in_volatile(&kit_in(Some(volatile.clone())), &StdLaunchFs, |fs| {
                match &write {
                    Write::File(path) => fs.write_file(path, b"x"),
                    Write::Link(path) => fs.link_dir(path, &elsewhere),
                }
                .map_err(|e| e.to_string())
            });
            assert_eq!(placed.is_err(), refused, "{what}: {placed:?}");
            library_untouched(what);
        }

        let settings = volatile.join("engram").join("settings.json");
        assert_eq!(
            place_in_volatile(&kit_in(Some(volatile.clone())), &StdLaunchFs, |fs| {
                fs.write_file(&settings, b"{}").map_err(|e| e.to_string())
            }),
            Ok(()),
            "even the write outside the link was blocked"
        );
        assert_eq!(std::fs::read(&settings).unwrap(), b"{}");
    }

    // ── Reply rows · notice fingerprint ──

    // ADR-0341
    #[test]
    fn the_fingerprint_is_the_omission_identity_only() {
        let x = || {
            vec![
                reason(
                    "omitted",
                    json!({ "names": ["b", "a"], "kind": "skill" }),
                    Some(3),
                ),
                reason("other", json!({}), None),
            ]
        };
        let base = notice_fingerprint(&x());

        let mut moved = x();
        moved[0].line = Some(40);
        moved[1].line = Some(1);
        assert_eq!(
            notice_fingerprint(&moved),
            base,
            "a `line` shift is the same set"
        );

        let mut reordered = x();
        reordered[0].args = json!({ "kind": "skill", "names": ["a", "b"] });
        assert_eq!(
            notice_fingerprint(&reordered),
            base,
            "a reordered key or name list is the same set"
        );

        let mut swapped = x();
        swapped.reverse();
        swapped.push(x()[0].clone());
        assert_eq!(
            notice_fingerprint(&swapped),
            base,
            "the set is sorted and deduplicated"
        );

        let mut renamed = x();
        renamed[0].args = json!({ "names": ["a", "c"], "kind": "skill" });
        assert_ne!(notice_fingerprint(&renamed), base, "another omitted name");
        let mut recoded = x();
        recoded[1].code = "another".to_string();
        assert_ne!(notice_fingerprint(&recoded), base, "another code");
        assert_ne!(notice_fingerprint(&x()[..1]), base, "a notice fewer");

        assert_eq!(
            notice_fingerprint(&[]),
            "",
            "the empty set is the empty string"
        );
        assert_ne!(notice_fingerprint(&[reason("", json!(null), None)]), "");
    }

    /// An argument cannot spell the separator of two notices — the join stays unambiguous.
    #[test]
    fn an_argument_holding_a_newline_does_not_forge_two_notices() {
        let two = [reason("a", json!(1), None), reason("b", json!(2), None)];
        let joined = notice_fingerprint(&two);
        let forged = [reason("a", json!(format!("1]\n{}", "[\"b\",2")), None)];
        assert_ne!(notice_fingerprint(&forged), joined);
        assert!(!notice_fingerprint(&forged).contains('\n'));
    }

    #[test]
    fn a_notice_row_carries_code_args_line_and_text() {
        let with_line = reason("omitted", json!({ "names": ["instructions.md"] }), Some(7));
        assert_eq!(
            notice_row(&with_line),
            json!({
                "code": "omitted",
                "args": { "names": ["instructions.md"] },
                "line": 7,
                "text": "Notice (omitted): intended behavior. Target — instructions.md",
            })
        );
        let without = reason("bare", json!({}), None);
        assert_eq!(
            notice_row(&without),
            json!({ "code": "bare", "args": {}, "text": "Notice (bare): intended behavior." }),
            "no `line` key when the notice names none"
        );
    }

    #[test]
    fn the_notice_text_lists_the_argument_values_in_key_order() {
        let n = reason(
            "omitted",
            json!({ "skills": ["s1", "s2"], "instructions": ["i.md"], "count": 2, "none": null }),
            None,
        );
        assert_eq!(
            notice_text(&n),
            "Notice (omitted): intended behavior. Target — 2 · i.md · s1 · s2"
        );
    }

    /// Outside names reach a line the CLI prints as is — a newline or a terminal escape in them must not.
    #[test]
    fn the_notice_text_escapes_control_characters() {
        let n = reason(
            "omit\nted",
            json!({ "names": ["a\nb.md", "\u{1b}[31mred", "tab\tname"] }),
            None,
        );
        let text = notice_text(&n);
        assert_eq!(
            text,
            "Notice (omit\\nted): intended behavior. Target — a\\nb.md · \\u{1b}[31mred · tab\\tname"
        );
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert_eq!(
            notice_row(&n)["args"],
            json!({ "names": ["a\nb.md", "\u{1b}[31mred", "tab\tname"] }),
            "the args stay raw — only the display line is escaped"
        );
    }

    /// The three codex notice codes give the user's wording exactly; an empty list leaves its part out.
    // ADR-0337
    // ADR-0341
    #[test]
    fn the_codex_notice_codes_give_the_exact_user_wording() {
        let cases = [
            (
                json!({ "instructions": ["team/a.md", "team/b.md"], "skills": ["review", "lint"] }),
                "Notice: codex terminal mode does not support preset instructions and skills. Not applied: instructions team/a.md, team/b.md, skills review, lint",
            ),
            (
                json!({ "instructions": [], "skills": ["a", "b"] }),
                "Notice: codex terminal mode does not support preset instructions and skills. Not applied: skills a, b",
            ),
            (
                json!({ "instructions": ["team/a.md"], "skills": [] }),
                "Notice: codex terminal mode does not support preset instructions and skills. Not applied: instructions team/a.md",
            ),
        ];
        for (args, text) in cases {
            assert_eq!(
                notice_text(&reason(CODEX_TERMINAL_OMITTED, args.clone(), None)),
                text,
                "{args}"
            );
        }

        assert_eq!(
            notice_text(&reason(
                CODEX_TERMINAL_FOLDER_OMITTED,
                json!({ "folder": "D:\\rules\\company", "items": ["AGENTS.md", ".agents\\skills"] }),
                None,
            )),
            "Notice: codex terminal mode does not support folder entries. Folder not applied: D:\\rules\\company (AGENTS.md, .agents\\skills)"
        );
        assert_eq!(
            notice_text(&reason(
                CODEX_SAME_NAME_SKILLS,
                json!({ "name": "review", "paths": ["D:\\lib\\review", "C:\\Users\\u\\.agents\\skills\\review"] }),
                None,
            )),
            "Notice: skills with the same name exist in several places. Calling $review in codex selects none of them. review: D:\\lib\\review, C:\\Users\\u\\.agents\\skills\\review"
        );
    }

    /// Names and paths in the codex wording are outside input — control characters are escaped there too; `args` of
    /// another shape fall back to the generic wording, and every other code keeps it.
    #[test]
    fn the_codex_notice_wording_escapes_names_and_falls_back_on_another_shape() {
        let text = notice_text(&reason(
            CODEX_SAME_NAME_SKILLS,
            json!({ "name": "a\nb", "paths": ["\u{1b}[31mred"] }),
            None,
        ));
        assert_eq!(
            text,
            "Notice: skills with the same name exist in several places. Calling $a\\nb in codex selects none of them. a\\nb: \\u{1b}[31mred"
        );
        assert!(!text.chars().any(char::is_control), "{text:?}");

        assert_eq!(
            notice_text(&reason(
                CODEX_TERMINAL_FOLDER_OMITTED,
                json!({ "items": ["x"] }),
                None
            )),
            "Notice (codex-terminal-folder-omitted): intended behavior. Target — x"
        );
        assert_eq!(
            notice_text(&reason(
                CODEX_TERMINAL_OMITTED,
                json!({ "skills": "one" }),
                None
            )),
            "Notice (codex-terminal-omitted): intended behavior. Target — one"
        );
        for args in [
            json!({ "name": "review" }),
            json!({ "name": "review", "paths": [] }),
        ] {
            assert_eq!(
                notice_text(&reason(CODEX_SAME_NAME_SKILLS, args.clone(), None)),
                "Notice (codex-same-name-skills): intended behavior. Target — review",
                "{args}"
            );
        }
        for args in [json!(["team/a.md"]), json!("x"), json!(null)] {
            let text = notice_text(&reason(CODEX_TERMINAL_OMITTED, args.clone(), None));
            assert!(
                text.starts_with("Notice (codex-terminal-omitted): intended behavior."),
                "{args} · {text}"
            );
        }
        assert_eq!(
            notice_text(&reason("codex-other", json!({ "name": "x" }), None)),
            "Notice (codex-other): intended behavior. Target — x"
        );
    }

    /// The folder-skip code names at most three paths and counts the rest, with control characters escaped; `args` of
    /// another shape fall back to the generic wording.
    #[test]
    fn the_folder_skip_notice_names_three_paths_counts_the_rest_and_falls_back_on_another_shape() {
        for (args, text) in [
            (
                json!({ "folder": "D:\\rules", "paths": ["a", "b/c"] }),
                "Notice: folder D:\\rules: skipped unreadable entries: a, b/c",
            ),
            (
                json!({ "folder": "D:\\rules", "paths": ["a", "b", "c"] }),
                "Notice: folder D:\\rules: skipped unreadable entries: a, b, c",
            ),
            (
                json!({ "folder": "D:\\rules", "paths": ["a", "b", "c", "d", "e"] }),
                "Notice: folder D:\\rules: skipped unreadable entries: a, b, c and 2 more",
            ),
            (
                json!({ "folder": "D:\\a\nb", "paths": ["x\u{1b}y"] }),
                "Notice: folder D:\\a\\nb: skipped unreadable entries: x\\u{1b}y",
            ),
        ] {
            let rendered = notice_text(&reason(FOLDER_LISTING_SKIPPED, args.clone(), None));
            assert!(!rendered.chars().any(char::is_control), "{rendered:?}");
            assert_eq!(rendered, text, "{args}");
        }
        let many: Vec<String> = (0..1_006).map(|n| format!("p{n}")).collect();
        assert_eq!(
            notice_text(&reason(
                FOLDER_LISTING_SKIPPED,
                json!({ "folder": "D:\\rules", "paths": many }),
                None
            )),
            "Notice: folder D:\\rules: skipped unreadable entries: p0, p1, p2 and 1,003 more"
        );
        for args in [
            json!({ "paths": ["a"] }),
            json!({ "folder": "D:\\x" }),
            json!({ "folder": "D:\\x", "paths": [] }),
            json!({ "folder": "D:\\x", "paths": [1] }),
            json!({ "folder": "D:\\x", "paths": "a" }),
            json!("D:\\x"),
        ] {
            let text = notice_text(&reason(FOLDER_LISTING_SKIPPED, args.clone(), None));
            assert!(
                text.starts_with("Notice (folder-listing-skipped): intended behavior."),
                "{args} · {text}"
            );
        }
    }

    /// The folder-listing code gives its own sentence, the folder from `args` with control characters escaped;
    /// `args` of another shape fall back to the generic wording.
    #[test]
    fn the_folder_listing_notice_gives_its_own_wording_and_falls_back_on_another_shape() {
        for (args, text) in [
            (
                json!({ "folder": "D:\\rules\\big" }),
                "Notice: folder D:\\rules\\big has more than 10,000 entries; only some were checked.",
            ),
            (
                json!({ "folder": "D:\\a\nb" }),
                "Notice: folder D:\\a\\nb has more than 10,000 entries; only some were checked.",
            ),
        ] {
            let rendered = notice_text(&reason(FOLDER_LISTING_PARTIAL, args.clone(), None));
            assert!(!rendered.chars().any(char::is_control), "{rendered:?}");
            assert_eq!(rendered, text, "{args}");
        }
        for args in [json!({ "path": "D:\\x" }), json!("D:\\x")] {
            let text = notice_text(&reason(FOLDER_LISTING_PARTIAL, args.clone(), None));
            assert!(
                text.starts_with("Notice (folder-listing-partial): intended behavior."),
                "{args} · {text}"
            );
        }
    }

    /// The built-in code gives its own sentence, the path and why from `args` with control characters escaped; `args`
    /// of another shape fall back to the generic wording.
    #[test]
    fn the_builtin_unavailable_notice_gives_its_own_wording_and_falls_back_on_another_shape() {
        for (args, text) in [
            (
                json!({ "path": "engram:/default", "why": "yaml-syntax: bad" }),
                "Notice: could not read built-in engram:/default; applied it with empty content. Reason: yaml-syntax: bad",
            ),
            (
                json!({ "path": "engram:/priming", "why": "a\nb\u{1b}" }),
                "Notice: could not read built-in engram:/priming; applied it with empty content. Reason: a\\nb\\u{1b}",
            ),
        ] {
            let rendered = notice_text(&reason(BUILTIN_UNAVAILABLE, args.clone(), None));
            assert!(!rendered.chars().any(char::is_control), "{rendered:?}");
            assert_eq!(rendered, text, "{args}");
        }
        for args in [
            json!({ "path": "engram:/default" }),
            json!({ "why": "no `kind`" }),
            json!({ "path": "engram:/default", "why": 1 }),
            json!({ "path": ["engram:/default"], "why": "x" }),
            json!("engram:/default"),
        ] {
            let text = notice_text(&reason(BUILTIN_UNAVAILABLE, args.clone(), None));
            assert!(
                text.starts_with("Notice (builtin-unavailable): intended behavior."),
                "{args} · {text}"
            );
        }
    }

    /// No settings code has its own sentence: the codes a settings schema check would emit take the generic wording
    /// like any unnamed code (ADR-0359 — no such check exists).
    #[test]
    fn the_former_settings_schema_codes_take_the_generic_wording() {
        for code in [
            "settings-schema-unavailable",
            "settings-schema-outdated",
            "settings-unknown-key",
        ] {
            assert_eq!(
                notice_text(&reason(
                    code,
                    json!({ "settings": "team/models.yaml" }),
                    None
                )),
                format!("Notice ({code}): intended behavior. Target — team/models.yaml")
            );
        }
    }

    // ── launch scope ──

    /// Counts `check` and `prepare` and keeps every default scope method.
    #[derive(Default)]
    struct DelegatingKits {
        checks: Mutex<usize>,
        prepares: Mutex<usize>,
    }

    impl LaunchKitSource for DelegatingKits {
        fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
            *self.checks.lock().unwrap() += 1;
            NoopLaunchKits.check(profile)
        }
        fn prepare(
            &self,
            plan: LaunchPlan,
            id: AgentId,
            epoch: u32,
        ) -> Result<LaunchKit, PrepareError> {
            *self.prepares.lock().unwrap() += 1;
            NoopLaunchKits.prepare(plan, id, epoch)
        }
        fn release(&self, _id: AgentId, _epoch: u32) {}
    }

    #[test]
    fn the_default_scope_methods_delegate_to_check_and_prepare() {
        let kits = DelegatingKits::default();
        let scope = kits.begin_launch();
        assert!(scope.get::<u32>().is_none(), "the default scope is empty");
        let p = profile(claude());

        let plan = kits.check_in(&scope, &p).unwrap();
        assert_eq!(plan, NoopLaunchKits.check(&p).unwrap());
        assert_eq!(*kits.checks.lock().unwrap(), 1);

        let kit = kits.prepare_in(&scope, plan.clone(), p.id, 2).unwrap();
        assert_eq!(kit, NoopLaunchKits.prepare(plan, p.id, 2).unwrap());
        assert_eq!(*kits.prepares.lock().unwrap(), 1);
    }

    /// The default `working_folder` is the `dunce` canonical path of a folder that exists
    /// (a `..` step resolved), the path as given for one that does not.
    #[test]
    fn the_default_working_folder_is_the_dunce_canonical_path_or_the_path_as_given() {
        let real = std::env::temp_dir();
        let dotted = real
            .join("..")
            .join(real.file_name().expect("the temp folder has a name"));
        let gone = real.join("engram-launch-scope-no-such-folder");
        assert!(!gone.exists(), "the sample must not exist");
        let scope = NoopLaunchKits.begin_launch();

        let canonical = NoopLaunchKits.working_folder(&scope, &dotted);
        assert_eq!(canonical, dunce::canonicalize(&real).unwrap());
        assert!(!canonical.components().any(|c| c == Component::ParentDir));
        assert_eq!(NoopLaunchKits.working_folder(&scope, &gone), gone);
    }

    #[test]
    fn a_scope_gives_its_value_only_as_its_own_type() {
        let value = Arc::new(7_u64);
        let scope = LaunchScope::new(value.clone());
        let got = scope.get::<u64>().expect("the scope holds a u64");
        assert!(Arc::ptr_eq(&got, &value));
        assert!(scope.get::<u32>().is_none());
        assert!(scope.get::<String>().is_none());
        assert!(Arc::ptr_eq(&scope.clone().get::<u64>().unwrap(), &value));
        assert!(LaunchScope::default().get::<u64>().is_none());
    }

    #[test]
    fn blocked_args_carry_every_reason_as_a_kind_row_in_order() {
        let blocked = LaunchBlocked {
            reasons: vec![
                BlockReason::Missing {
                    item: "folder:D:\\rules".to_string(),
                    via: vec!["team/base.yaml".to_string()],
                    reason: reason("missing-folder", json!({ "path": "D:\\rules" }), None),
                },
                BlockReason::Broken {
                    item: "team/a.yaml".to_string(),
                    via: Vec::new(),
                    reason: reason("yaml", json!({}), Some(2)),
                },
                BlockReason::Conflict {
                    reason: reason("same-skill", json!({ "name": "s" }), None),
                    items: vec![
                        ("x/s".to_string(), vec!["p1.yaml".to_string()]),
                        ("y/s".to_string(), vec!["p2.yaml".to_string()]),
                    ],
                },
                BlockReason::LibraryUnavailable {
                    reason: reason("no-library", json!({}), None),
                },
            ],
        };
        let args = blocked.to_args();
        assert_eq!(args.len(), 1, "only `reasons` — callers add their own keys");
        assert_eq!(
            Value::Object(args),
            json!({ "reasons": [
                { "kind": "missing", "item": "folder:D:\\rules", "via": ["team/base.yaml"],
                  "code": "missing-folder", "args": { "path": "D:\\rules" } },
                { "kind": "broken", "item": "team/a.yaml", "via": [],
                  "code": "yaml", "args": {}, "line": 2 },
                { "kind": "conflict",
                  "items": [ { "item": "x/s", "via": ["p1.yaml"] }, { "item": "y/s", "via": ["p2.yaml"] } ],
                  "code": "same-skill", "args": { "name": "s" } },
                { "kind": "library_unavailable", "code": "no-library", "args": {} },
            ] })
        );
    }
}
