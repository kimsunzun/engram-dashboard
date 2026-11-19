//! The library command port — `impl LibraryCommandHost for LibraryService`: the nine `library.*` commands agent
//! declares, answered by the bodies of this crate.
//!
//! ## Rules (invariants)
//!
//! - **One body per call, mapped as built.** Each method parses its spellings, calls its one body and converts the
//!   crate-local answer into agent's declared rows; it never calls a second body to rebuild an answer and never reads a
//!   file back (ADR-0384).
//! - **Spellings are parsed with the bodies' own rules** (`edit::lib_path`) — here for `refresh` / `list` / `read`, by
//!   the body for `newFolder` / `newFile` / `relink`: a `path` / `folder` / `parent` that is not a library path refuses
//!   `InvalidArgument` · `item-path-invalid`, nothing done.
//! - **Refusals cross 1:1.** [`LibraryRefusal`] becomes agent's `PortRefusal` with the same class and reason (or the
//!   same store error); agent picks the bus code (ADR-0379). `list`'s bare reason (`library-unavailable`) gets class
//!   `Unavailable`.
// ADR-0311
// ADR-0379
// ADR-0384
// ADR-0386

use std::path::Path;

use engram_dashboard_agent::commands::{
    self as declared, LibraryFolderRow, LibraryFoldersOk, LibraryKind, LibraryListOk,
    LibraryNewFileKind, LibraryRefreshOk, LibraryRelinkOk, LibraryStatusOk, ListFolderRow,
    PathReasonRow, ReasonRow, RefreshErrorRow, RelinkPairRow,
};
use engram_dashboard_agent::launch::Reason;
use engram_dashboard_agent::library_commands::{
    relink_stop_row, LibraryCommandHost, LibraryReadView, PortClass, PortRefusal,
};

use crate::catalog::ItemKind;
use crate::config::ConfigHealth;
use crate::create::NewFileKind;
use crate::edit::lib_path;
use crate::relink::RelinkAnswer;
use crate::{
    FoldersAnswer, LibraryRefusal, LibraryService, LibraryStatus, ListAnswer, ListFilter, ListRow,
    ReadAnswer, RefreshReport, RefusalClass, RootSettingKind, RootStatus,
};

impl LibraryCommandHost for LibraryService {
    fn status(&self) -> Result<LibraryStatusOk, PortRefusal> {
        Ok(status_ok(LibraryService::status(self)))
    }

    fn set_root(&self, path: &str, create: bool) -> Result<LibraryStatusOk, PortRefusal> {
        LibraryService::set_root(self, Path::new(path), create)
            .map(status_ok)
            .map_err(port)
    }

    fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal> {
        LibraryService::folders(self).map(folders_ok).map_err(port)
    }

    fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal> {
        let folder = folder.map(lib_path).transpose().map_err(port)?;
        Ok(refresh_ok(LibraryService::refresh(self, folder.as_ref())))
    }

    fn list(
        &self,
        folder: Option<&str>,
        query: Option<&str>,
        kinds: &[LibraryKind],
        builtins: bool,
    ) -> Result<LibraryListOk, PortRefusal> {
        let folder = folder.map(lib_path).transpose().map_err(port)?;
        let filter = ListFilter {
            query: query.map(str::to_owned),
            kinds: kinds.iter().map(item_kind).collect(),
            builtins,
        };
        self.list_with(folder.as_ref(), &filter)
            .map(list_ok)
            .map_err(|reason| PortRefusal::Refused {
                class: PortClass::Unavailable,
                reason,
            })
    }

    fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal> {
        let path = lib_path(path).map_err(port)?;
        LibraryService::read(self, &path)
            .map(read_view)
            .map_err(port)
    }

    fn new_folder(
        &self,
        parent: Option<&str>,
        name: &str,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        LibraryService::new_folder(self, parent, name, root_generation)
            .map(|path| path.as_str().to_owned())
            .map_err(port)
    }

    fn new_file(
        &self,
        folder: Option<&str>,
        name: &str,
        kind: LibraryNewFileKind,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        let kind = match kind {
            LibraryNewFileKind::Markdown => NewFileKind::Markdown,
            LibraryNewFileKind::Settings => NewFileKind::Settings,
            LibraryNewFileKind::Preset => NewFileKind::Preset,
        };
        LibraryService::new_file(self, folder, name, kind, root_generation)
            .map(|path| path.as_str().to_owned())
            .map_err(port)
    }

    fn relink(
        &self,
        from: &str,
        to: &str,
        root_generation: Option<u64>,
    ) -> Result<LibraryRelinkOk, PortRefusal> {
        LibraryService::relink(self, from, to, root_generation)
            .map(relink_ok)
            .map_err(port)
    }
}

/// [`LibraryRefusal`] → the port refusal, 1:1.
fn port(refusal: LibraryRefusal) -> PortRefusal {
    match refusal {
        LibraryRefusal::Refused { class, reason } => PortRefusal::Refused {
            class: match class {
                RefusalClass::Conflict => PortClass::Conflict,
                RefusalClass::NotFound => PortClass::NotFound,
                RefusalClass::InvalidArgument => PortClass::InvalidArgument,
                RefusalClass::Unavailable => PortClass::Unavailable,
                RefusalClass::Internal => PortClass::Internal,
            },
            reason,
        },
        LibraryRefusal::Store(error) => PortRefusal::Store(error),
    }
}

/// A count as the declared `u32` — a library never holds 2^32 of anything, so the cap is never reached in practice.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn status_ok(status: LibraryStatus) -> LibraryStatusOk {
    let (state, state_why) = match status.state {
        RootStatus::Unset => ("unset", None),
        RootStatus::Ok => ("ok", None),
        RootStatus::Missing { why } => ("missing", Some(why)),
    };
    let (config, config_why) = match status.config {
        ConfigHealth::Ok => ("ok", None),
        ConfigHealth::ReadOnly { why } => ("read_only", Some(why)),
        ConfigHealth::Corrupt => ("corrupt", None),
        ConfigHealth::Unsaved { why } => ("unsaved", Some(why)),
    };
    LibraryStatusOk {
        daemon_id: status.daemon_id,
        root: status.root.map(|root| root.display().to_string()),
        root_setting: status.root_setting.map(|setting| {
            match setting {
                RootSettingKind::Default => "default",
                RootSettingKind::Explicit => "explicit",
            }
            .to_owned()
        }),
        state: state.to_owned(),
        state_why,
        config: config.to_owned(),
        config_why,
        suggested_new: status.suggested_new.display().to_string(),
        root_generation: status.root_generation,
    }
}

fn folders_ok(answer: FoldersAnswer) -> LibraryFoldersOk {
    LibraryFoldersOk {
        folders: answer
            .folders
            .into_iter()
            .map(|folder| LibraryFolderRow {
                path: folder.path,
                name: folder.name,
                skill: folder.skill,
            })
            .collect(),
    }
}

fn refresh_ok(report: RefreshReport) -> LibraryRefreshOk {
    LibraryRefreshOk {
        changed: count(report.changed),
        problems: report
            .problems
            .into_iter()
            .map(|(path, reason)| path_reason(path.as_str().to_owned(), reason))
            .collect(),
        no_kind: report
            .no_kind
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        skipped_links: report
            .skipped_links
            .iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        errors: report
            .errors
            .into_iter()
            .map(|(path, why)| RefreshErrorRow {
                path: path.map(|path| path.as_str().to_owned()),
                why,
            })
            .collect(),
    }
}

fn list_ok(answer: ListAnswer) -> LibraryListOk {
    LibraryListOk {
        folder: answer.folder.map(|folder| folder.as_str().to_owned()),
        rows: answer.rows.into_iter().map(list_row).collect(),
        folders: answer
            .folders
            .into_iter()
            .map(|folder| ListFolderRow {
                path: folder.path,
                name: folder.name,
            })
            .collect(),
        builtins: answer.builtins.into_iter().map(list_row).collect(),
    }
}

fn read_view(answer: ReadAnswer) -> LibraryReadView {
    LibraryReadView {
        row: list_row(answer.row),
        absolute: answer.absolute.display().to_string(),
        presets_direct: answer.presets_direct,
        presets_indirect: answer.presets_indirect,
        agents_direct: answer.agents_direct,
        agents_indirect: answer.agents_indirect,
        excerpt: answer.excerpt,
        settings_fields: answer.settings_fields,
    }
}

fn relink_ok(answer: RelinkAnswer) -> LibraryRelinkOk {
    LibraryRelinkOk {
        fixed: answer
            .fixed
            .into_iter()
            .map(|(from, to)| RelinkPairRow { from, to })
            .collect(),
        more: count(answer.more),
        written: answer.written,
        skipped: answer
            .skipped
            .into_iter()
            .map(|skip| path_reason(skip.path, skip.reason))
            .collect(),
        stopped: answer
            .stopped
            .map(|stop| relink_stop_row(stop.path, port(stop.refusal))),
    }
}

fn list_row(row: ListRow) -> declared::ListRow {
    declared::ListRow {
        path: row.path,
        kind: row.kind.listed().as_ref().and_then(library_kind),
        name: row.name,
        state: row.state.map(reason_row),
        references: count(row.references),
        description: row.description,
    }
}

fn reason_row(reason: Reason) -> ReasonRow {
    ReasonRow {
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

fn path_reason(path: String, reason: Reason) -> PathReasonRow {
    PathReasonRow {
        path,
        code: reason.code,
        args: reason.args,
        line: reason.line,
    }
}

/// A listed kind ([`ItemKind::listed`]) as the declared word; `Broken` has none.
fn library_kind(kind: &ItemKind) -> Option<LibraryKind> {
    match kind {
        ItemKind::Instruction => Some(LibraryKind::Instruction),
        ItemKind::Skill => Some(LibraryKind::Skill),
        ItemKind::SkillFile => Some(LibraryKind::SkillFile),
        ItemKind::Settings => Some(LibraryKind::Settings),
        ItemKind::Preset => Some(LibraryKind::Preset),
        ItemKind::Broken { .. } => None,
    }
}

fn item_kind(kind: &LibraryKind) -> ItemKind {
    match kind {
        LibraryKind::Instruction => ItemKind::Instruction,
        LibraryKind::Skill => ItemKind::Skill,
        LibraryKind::SkillFile => ItemKind::SkillFile,
        LibraryKind::Settings => ItemKind::Settings,
        LibraryKind::Preset => ItemKind::Preset,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use engram_dashboard_agent::profile::{
        AgentCommand, AgentProfile, AttachedPreset, ProfileRegistry, ProfileStore, Refusal,
        StoreError,
    };
    use engram_dashboard_agent::types::AgentId;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::config::{LibraryConfig, RootSetting};
    use crate::folder::{BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP};
    use crate::format::read::ITEM_PATH_INVALID;
    use crate::relink::{RelinkSkip, RelinkStop};
    use crate::store::{FsStore, LibraryStore};
    use crate::{LibraryValues, LIBRARY_UNAVAILABLE, PATH_THROUGH_LINK};
    use engram_dashboard_platform::fs::{link_dir, unlink_dir};

    struct Fixed(Vec<AgentProfile>);

    impl ProfileStore for Fixed {
        fn save(&self, _: &[AgentProfile]) -> Result<(), StoreError> {
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.0.clone()
        }
    }

    fn agent(presets: &[&str], default_preset: bool) -> AgentProfile {
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
        profile.default_preset = default_preset;
        profile.presets = presets
            .iter()
            .map(|path| AttachedPreset {
                path: (*path).to_owned(),
            })
            .collect();
        profile
    }

    /// A temp folder holding the library root `lib\` (published when `root`) over the real store, a registry over the
    /// profiles, no built-in folder and no profile-edit port.
    struct World {
        _temp: TempDir,
        base: PathBuf,
        svc: Arc<LibraryService>,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let link = self.base.join("lib").join("team");
            if fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink()) {
                if let Err(e) = unlink_dir(&link) {
                    eprintln!("could not remove the test link {}: {e}", link.display());
                }
            }
        }
    }

    impl World {
        fn new(profiles: Vec<AgentProfile>) -> World {
            World::make(profiles, true)
        }

        fn make(profiles: Vec<AgentProfile>, root: bool) -> World {
            let temp = tempfile::tempdir().unwrap();
            let base = dunce::canonicalize(temp.path()).unwrap();
            fs::create_dir_all(base.join("lib")).unwrap();
            let state = base.join("state");
            fs::create_dir_all(&state).unwrap();
            let config = match root {
                true => LibraryConfig::usable(&state, RootSetting::Explicit(base.join("lib"))),
                false => LibraryConfig::missing(&state),
            };
            let values = LibraryValues {
                registry: Arc::new(ProfileRegistry::new(Arc::new(Fixed(profiles)))),
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
            let svc = Arc::new(LibraryService::new(
                values,
                Arc::new(FsStore) as Arc<dyn LibraryStore>,
                io,
            ));
            World {
                _temp: temp,
                base,
                svc,
            }
        }

        fn put(&self, rel: &str, text: &str) {
            let path = self.base.join("lib").join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        /// The service through the port, as the daemon table holds it.
        fn port(&self) -> &dyn LibraryCommandHost {
            &*self.svc
        }

        fn list(
            &self,
            folder: Option<&str>,
            query: Option<&str>,
            kinds: &[LibraryKind],
        ) -> LibraryListOk {
            self.port()
                .list(folder, query, kinds, false)
                .unwrap_or_else(|e| panic!("list: {e:?}"))
        }
    }

    fn refused<T: std::fmt::Debug>(answer: Result<T, PortRefusal>) -> (PortClass, Reason) {
        match answer {
            Err(PortRefusal::Refused { class, reason }) => (class, reason),
            other => panic!("expected a refusal: {other:?}"),
        }
    }

    fn paths(rows: &[declared::ListRow]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    const SKILL: &str = "---\nname: s1\ndescription: Says hello\n---\n";

    /// `library.status` and `library.setRoot` carry the body's status field by field, in the declared words.
    #[test]
    fn status_and_set_root_carry_the_body_status_field_by_field() {
        let w = World::new(Vec::new());
        let body = w.svc.status();
        let out = w.port().status().expect("status");
        assert_eq!(
            out,
            LibraryStatusOk {
                daemon_id: "test-daemon".to_owned(),
                root: Some(w.base.join("lib").display().to_string()),
                root_setting: Some("explicit".to_owned()),
                state: "ok".to_owned(),
                state_why: None,
                config: "ok".to_owned(),
                config_why: None,
                suggested_new: w.base.join("default-library").display().to_string(),
                root_generation: body.root_generation,
            }
        );

        let other = w.base.join("other");
        let moved = w
            .port()
            .set_root(&other.display().to_string(), true)
            .expect("set root");
        let after = w.svc.status();
        assert_eq!(moved.root, Some(other.display().to_string()));
        assert_eq!(moved.state, "ok");
        assert_eq!(moved.root_generation, after.root_generation);
        assert_ne!(moved.root_generation, body.root_generation);
    }

    /// With no root the status says `unset` with no root and no reason.
    #[test]
    fn an_unset_root_reads_unset() {
        let w = World::make(Vec::new(), false);
        let out = w.port().status().expect("status");
        assert_eq!(
            (out.root, out.state.as_str(), out.state_why),
            (None, "unset", None)
        );
    }

    /// Every path argument that is not a library path refuses `InvalidArgument` · `item-path-invalid` with nothing
    /// done — a built-in spelling included (it is not a library path).
    #[test]
    fn every_path_argument_that_is_not_a_library_path_is_refused_item_path_invalid() {
        let w = World::new(Vec::new());
        let port = w.port();
        let cases: Vec<(&str, (PortClass, Reason))> = vec![
            ("read", refused(port.read("../x.md"))),
            ("read built-in", refused(port.read("engram:/default"))),
            ("list", refused(port.list(Some("a/../b"), None, &[], false))),
            ("refresh", refused(port.refresh(Some("/abs")))),
            (
                "newFolder",
                refused(port.new_folder(Some("a\\b"), "x", None)),
            ),
            (
                "newFile",
                refused(port.new_file(Some(".."), "x", LibraryNewFileKind::Markdown, None)),
            ),
            ("relink from", refused(port.relink("..", "a.md", None))),
            ("relink to", refused(port.relink("a.md", "b/", None))),
        ];
        for (what, (class, reason)) in cases {
            assert_eq!(
                (class, reason.code.as_str()),
                (PortClass::InvalidArgument, ITEM_PATH_INVALID),
                "{what}"
            );
        }
        let (class, reason) = refused(port.set_root("relative/lib", false));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::InvalidArgument, ITEM_PATH_INVALID)
        );
    }

    /// `list`'s bare reason gets the class `Unavailable`; the other catalog reads answer theirs as the body does.
    #[test]
    fn reads_without_a_library_refuse_unavailable() {
        let w = World::make(Vec::new(), false);
        let (class, reason) = refused(w.port().list(None, None, &[], false));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Unavailable, LIBRARY_UNAVAILABLE)
        );
        let (class, reason) = refused(w.port().folders());
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Unavailable, LIBRARY_UNAVAILABLE)
        );
    }

    /// A refresh that cannot scan is a report, never a refusal: one `errors` row with no path.
    #[test]
    fn a_refresh_with_no_root_answers_one_error_row_with_no_path() {
        let w = World::make(Vec::new(), false);
        let out = w.port().refresh(None).expect("a report");
        assert_eq!(out.errors.len(), 1, "{out:?}");
        assert_eq!(out.errors[0].path, None);
        assert_eq!((out.changed, out.problems.len()), (0, 0));
    }

    /// The report is carried as built: each problem keeps its path, code, args and line; links, no-kind files and
    /// errors keep their paths.
    #[test]
    fn a_refresh_report_is_carried_as_built() {
        let lp = |text: &str| crate::path::LibPath::parse(text).unwrap();
        let report = RefreshReport {
            changed: 3,
            problems: vec![(
                lp("p.yaml"),
                Reason {
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(2),
                },
            )],
            no_kind: vec![lp("n.yaml")],
            skipped_links: vec![lp("team")],
            errors: vec![
                (Some(lp("d")), "denied".to_owned()),
                (None, "gone".to_owned()),
            ],
        };
        assert_eq!(
            refresh_ok(report),
            LibraryRefreshOk {
                changed: 3,
                problems: vec![PathReasonRow {
                    path: "p.yaml".to_owned(),
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(2),
                }],
                no_kind: vec!["n.yaml".to_owned()],
                skipped_links: vec!["team".to_owned()],
                errors: vec![
                    RefreshErrorRow {
                        path: Some("d".to_owned()),
                        why: "denied".to_owned(),
                    },
                    RefreshErrorRow {
                        path: None,
                        why: "gone".to_owned(),
                    },
                ],
            }
        );
    }

    /// A skill folder lists as `Skill` in its parent; its `SKILL.md` lists inside it as `SkillFile`, and `kinds`
    /// matches that row only by `SkillFile` — `Skill` does not.
    #[test]
    fn a_skill_file_row_is_matched_only_by_skill_file() {
        let w = World::new(Vec::new());
        w.put("skills/s1/SKILL.md", SKILL);
        let parent = w.list(Some("skills"), None, &[LibraryKind::Skill]);
        assert_eq!(paths(&parent.rows), ["skills/s1"]);
        assert_eq!(parent.rows[0].kind, Some(LibraryKind::Skill));
        assert_eq!(parent.rows[0].name, "s1");

        let inside = w.list(Some("skills/s1"), None, &[]);
        assert_eq!(paths(&inside.rows), ["skills/s1/SKILL.md"]);
        assert_eq!(inside.rows[0].kind, Some(LibraryKind::SkillFile));
        let by_file = w.list(Some("skills/s1"), None, &[LibraryKind::SkillFile]);
        assert_eq!(paths(&by_file.rows), ["skills/s1/SKILL.md"]);
        let by_skill = w.list(Some("skills/s1"), None, &[LibraryKind::Skill]);
        assert!(by_skill.rows.is_empty(), "{by_skill:?}");
    }

    /// A query searches name and description, case-insensitively, in every item below the folder (not the folder
    /// itself), and the answer carries matching rows only — no folder lines.
    #[test]
    fn a_query_searches_every_item_below_the_folder_and_answers_no_folder_lines() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "Hello there");
        w.put("rules/deep/hello-b.md", "x");
        w.put("rules/deep/c.md", "nothing");
        w.put("other.md", "HELLO");
        w.put("skills/s1/SKILL.md", SKILL);

        let all = w.list(None, Some("hello"), &[]);
        assert_eq!(
            paths(&all.rows),
            [
                "other.md",
                "rules/a.md",
                "rules/deep/hello-b.md",
                "skills/s1"
            ]
        );
        assert!(all.folders.is_empty(), "{all:?}");

        let below = w.list(Some("rules/deep"), Some("HELLO"), &[]);
        assert_eq!(paths(&below.rows), ["rules/deep/hello-b.md"]);

        let parent = w.list(Some("skills"), Some("s1"), &[]);
        assert_eq!(paths(&parent.rows), ["skills/s1"]);
        let itself = w.list(Some("skills/s1"), Some("s1"), &[]);
        assert!(
            itself.rows.is_empty(),
            "the folder asked for is not its own result: {itself:?}"
        );

        let plain = w.list(Some("rules"), None, &[]);
        assert_eq!(paths(&plain.rows), ["rules/a.md"]);
        assert_eq!(
            plain.folders.len(),
            1,
            "without a query the folder lines stay"
        );
    }

    /// `kinds` keeps a broken file by the kind it claims; a broken file that claims none matches no kind.
    #[test]
    fn kinds_keep_a_broken_file_by_the_kind_it_claims() {
        let w = World::new(Vec::new());
        w.put("bad.yaml", "kind: preset\nversion: 1\ninstructions: [\n");
        w.put("odd.yaml", "kind: gadget\nversion: 1\n");
        w.put("good.yaml", "kind: preset\nversion: 1\ndescription: fine\n");
        w.put("a.md", "x");

        let every = w.list(None, None, &[]);
        let bad = every
            .rows
            .iter()
            .find(|row| row.path == "bad.yaml")
            .unwrap();
        assert_eq!(bad.kind, Some(LibraryKind::Preset));
        assert!(bad.state.is_some(), "a broken file carries its reason");
        let odd = every
            .rows
            .iter()
            .find(|row| row.path == "odd.yaml")
            .unwrap();
        assert_eq!(odd.kind, None);

        let presets = w.list(None, None, &[LibraryKind::Preset]);
        assert_eq!(paths(&presets.rows), ["bad.yaml", "good.yaml"]);
        let both = w.list(None, None, &[LibraryKind::Preset, LibraryKind::Instruction]);
        assert_eq!(paths(&both.rows), ["a.md", "bad.yaml", "good.yaml"]);
    }

    /// `builtins` answers the built-in preset row — its spelling, its id as the name, a stand-in's reason, and the
    /// agents whose Default slot is on — under the same query and kinds; without it, none.
    #[test]
    fn builtins_answer_the_default_preset_row_under_the_same_filters() {
        let w = World::new(vec![
            agent(&[], true),
            agent(&[], true),
            agent(&["p.yaml"], false),
        ]);
        let none = w.list(None, None, &[]);
        assert!(none.builtins.is_empty());

        let with = w.port().list(None, None, &[], true).expect("list");
        assert_eq!(with.builtins.len(), 1, "{with:?}");
        let row = &with.builtins[0];
        assert_eq!(row.path, "engram:/default");
        assert_eq!(row.name, "default");
        assert_eq!(row.kind, Some(LibraryKind::Preset));
        assert_eq!(row.references, 2);
        let state = row.state.as_ref().expect("no built-in folder: a stand-in");
        assert_eq!(
            state.code,
            engram_dashboard_agent::launch::BUILTIN_UNAVAILABLE
        );

        let kept = w
            .port()
            .list(None, Some("DEF"), &[LibraryKind::Preset], true);
        assert_eq!(kept.expect("list").builtins.len(), 1);
        let by_kind = w.port().list(None, None, &[LibraryKind::Instruction], true);
        assert!(by_kind.expect("list").builtins.is_empty());
        let by_query = w.port().list(None, Some("zzz"), &[], true);
        assert!(by_query.expect("list").builtins.is_empty());
    }

    /// `library.read` carries the body's answer field by field, agents by id.
    #[test]
    fn read_carries_the_body_answer_field_by_field() {
        let attached = agent(&["p.yaml"], false);
        let id: AgentId = attached.id;
        let w = World::new(vec![attached]);
        w.put("rules/a.md", "Body text");
        w.put(
            "p.yaml",
            "kind: preset\nversion: 1\ninstructions:\n  - rules/a.md\n",
        );
        let view = w.port().read("rules/a.md").expect("read");
        let body = w
            .svc
            .read(&crate::path::LibPath::parse("rules/a.md").unwrap())
            .expect("body read");
        assert_eq!(view.absolute, body.absolute.display().to_string());
        assert_eq!(view.presets_direct, ["p.yaml"]);
        assert_eq!(view.presets_direct, body.presets_direct);
        assert_eq!(view.presets_indirect, body.presets_indirect);
        assert_eq!(view.agents_direct, body.agents_direct);
        assert_eq!(view.agents_indirect, [id]);
        assert_eq!(view.excerpt, body.excerpt);
        assert_eq!(view.settings_fields, body.settings_fields);
        assert_eq!(view.row.path, "rules/a.md");
        assert_eq!(view.row.kind, Some(LibraryKind::Instruction));
        assert_eq!(view.row.references, 1);
        assert_eq!(view.row.description, body.row.description);

        let (class, reason) = refused(w.port().read("rules/none.md"));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::NotFound, crate::NOT_FOUND)
        );
    }

    /// The creates answer the path the body made, and a taken name is the body's refusal.
    #[test]
    fn creates_answer_the_path_the_body_made() {
        let w = World::new(Vec::new());
        let folder = w.port().new_folder(None, "team", None).expect("folder");
        assert_eq!(folder, "team");
        let file = w
            .port()
            .new_file(Some("team"), "p", LibraryNewFileKind::Preset, None)
            .expect("file");
        assert_eq!(file, "team/p.yaml");
        assert!(w.base.join("lib").join("team").join("p.yaml").is_file());
        let (class, reason) = refused(w.port().new_folder(None, "team", None));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::create::NAME_EXISTS)
        );
        let (class, reason) = refused(w.port().new_folder(None, "x", Some(1)));
        assert_eq!(
            (class, reason.code.as_str()),
            (PortClass::Conflict, crate::ROOT_GENERATION_STALE)
        );
    }

    /// A relink whose agents need the profile-edit port while none is plugged refuses `Internal` ·
    /// `profile-edit-unavailable` — the one class that keeps the request id.
    #[test]
    fn a_relink_with_no_profile_edit_port_refuses_internal() {
        let w = World::new(vec![agent(&["p.yaml"], false)]);
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        w.put("q/p.yaml", "kind: preset\nversion: 1\n");
        let (class, reason) = refused(w.port().relink("p.yaml", "q/p.yaml", None));
        assert_eq!(
            (class, reason.code.as_str()),
            (
                PortClass::Internal,
                crate::agent_edit::PROFILE_EDIT_UNAVAILABLE
            )
        );
    }

    /// The relink answer is carried as built: pairs, `more`, written files, skips with their full reason, and a stop
    /// as the refusal it met — a store error as `profile-store-refused`.
    #[test]
    fn a_relink_answer_is_carried_as_built() {
        let answer = RelinkAnswer {
            fixed: vec![("a.md".to_owned(), "b/a.md".to_owned())],
            more: 1,
            written: vec!["p.yaml".to_owned()],
            skipped: vec![RelinkSkip {
                path: "q.yaml".to_owned(),
                reason: Reason {
                    code: "yaml-syntax".to_owned(),
                    args: json!({ "message": "m" }),
                    line: Some(3),
                },
            }],
            stopped: Some(RelinkStop {
                path: None,
                refusal: LibraryRefusal::Store(StoreError::Refused(Refusal::Unreadable)),
            }),
        };
        let out = relink_ok(answer);
        assert_eq!(
            out.fixed,
            [RelinkPairRow {
                from: "a.md".to_owned(),
                to: "b/a.md".to_owned(),
            }]
        );
        assert_eq!(
            (out.more, out.written.as_slice()),
            (1, ["p.yaml".to_owned()].as_slice())
        );
        assert_eq!(
            out.skipped,
            [PathReasonRow {
                path: "q.yaml".to_owned(),
                code: "yaml-syntax".to_owned(),
                args: json!({ "message": "m" }),
                line: Some(3),
            }]
        );
        let stop = out.stopped.expect("stopped");
        assert_eq!(stop.path, None);
        assert_eq!(
            stop.code,
            engram_dashboard_agent::library_commands::PROFILE_STORE_REFUSED
        );
    }
    /// Each refusal the caller can clear and then repeat is in agent's `AfterCondition` list, under the code this
    /// crate answers — a renamed const would fall out of the directive silently.
    #[test]
    fn the_clearable_refusal_codes_are_in_the_after_condition_list() {
        use engram_dashboard_agent::library_commands::RETRY_AFTER_CONDITION;
        for code in [
            crate::LIBRARY_UNAVAILABLE,
            crate::edit::LIBRARY_WRITE_FAILED,
            crate::launch::LIBRARY_READ_FAILED,
            crate::LIBRARY_ROOT_UNUSABLE,
            crate::LIBRARY_CONFIG_WRITE_FAILED,
        ] {
            assert!(RETRY_AFTER_CONDITION.contains(&code), "{code}");
        }
        assert_eq!(RETRY_AFTER_CONDITION.len(), 5);
    }

    /// Every status word and reason lands in its own field: `missing` + `state_why`, each `config` word with its
    /// `config_why` (`corrupt` has none), and `root_setting` `default`.
    #[test]
    fn every_status_word_lands_in_its_own_field() {
        let status = |state: RootStatus, config: ConfigHealth| LibraryStatus {
            daemon_id: "d".to_owned(),
            root: Some(PathBuf::from("C:/lib")),
            root_setting: Some(RootSettingKind::Default),
            state,
            config,
            suggested_new: PathBuf::from("C:/data/library"),
            root_generation: 9,
        };
        let out = status_ok(status(
            RootStatus::Missing {
                why: "timed out".to_owned(),
            },
            ConfigHealth::ReadOnly {
                why: "newer".to_owned(),
            },
        ));
        assert_eq!(out.root_setting.as_deref(), Some("default"));
        assert_eq!(
            (out.state.as_str(), out.state_why.as_deref()),
            ("missing", Some("timed out"))
        );
        assert_eq!(
            (out.config.as_str(), out.config_why.as_deref()),
            ("read_only", Some("newer"))
        );
        let out = status_ok(status(
            RootStatus::Ok,
            ConfigHealth::Unsaved {
                why: "write failed".to_owned(),
            },
        ));
        assert_eq!(
            (out.config.as_str(), out.config_why.as_deref()),
            ("unsaved", Some("write failed"))
        );
        assert_eq!((out.state.as_str(), out.state_why), ("ok", None));
        let out = status_ok(status(RootStatus::Ok, ConfigHealth::Corrupt));
        assert_eq!((out.config.as_str(), out.config_why), ("corrupt", None));
    }

    /// `Settings` crosses both ways: a settings file lists as `Settings` and `kinds: [Settings]` keeps it, and
    /// `newFile {kind: Settings}` writes the settings template.
    #[test]
    fn the_settings_kind_crosses_both_ways() {
        let w = World::new(Vec::new());
        w.put("s.yaml", "kind: settings\nversion: 1\ndescription: d\n");
        w.put("p.yaml", "kind: preset\nversion: 1\n");
        let kept = w.list(None, None, &[LibraryKind::Settings]);
        assert_eq!(paths(&kept.rows), ["s.yaml"]);
        assert_eq!(kept.rows[0].kind, Some(LibraryKind::Settings));

        let made = w
            .port()
            .new_file(None, "fresh", LibraryNewFileKind::Settings, None)
            .expect("file");
        assert_eq!(made, "fresh.yaml");
        let text = fs::read_to_string(w.base.join("lib").join("fresh.yaml")).unwrap();
        assert!(text.starts_with("kind: settings\n"), "{text}");
    }

    /// `library.folders` answers every folder with the skill flag as the body set it.
    #[test]
    fn folders_carry_the_skill_flag() {
        let w = World::new(Vec::new());
        w.put("rules/a.md", "x");
        w.put("skills/s1/SKILL.md", SKILL);
        let out = w.port().folders().expect("folders");
        let rows: Vec<(&str, &str, bool)> = out
            .folders
            .iter()
            .map(|row| (row.path.as_str(), row.name.as_str(), row.skill))
            .collect();
        assert_eq!(
            rows,
            [
                ("rules", "rules", false),
                ("skills", "skills", false),
                ("skills/s1", "s1", true),
            ]
        );
    }

    /// A folder behind a link answers its one `path-through-link` row whatever the filters say, and still the
    /// built-in row the filters keep.
    #[test]
    fn a_folder_behind_a_link_answers_its_link_row_and_the_builtin_row() {
        let w = World::new(Vec::new());
        let outside = w.base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("x.md"), "x").unwrap();
        link_dir(&outside, &w.base.join("lib").join("team")).unwrap();

        let out = w
            .port()
            .list(Some("team/x.md"), Some("zzz"), &[LibraryKind::Skill], true)
            .expect("list");
        assert_eq!(paths(&out.rows), ["team/x.md"]);
        assert_eq!(
            out.rows[0].state.as_ref().map(|state| state.code.as_str()),
            Some(PATH_THROUGH_LINK)
        );
        assert!(out.builtins.is_empty(), "the query drops the built-in row");

        let out = w
            .port()
            .list(Some("team/x.md"), None, &[], true)
            .expect("list");
        assert_eq!(paths(&out.rows), ["team/x.md"]);
        assert_eq!(paths(&out.builtins), ["engram:/default"]);
    }
}
