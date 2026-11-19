//! `library.*` handlers — the port asset-library implements ([`LibraryCommandHost`]) and the one mapping of a port
//! refusal onto a command error ([`refusal_error`]). The declarations are in the one `declare_commands!` block of
//! `commands.rs` (one block per module).
//!
//! ## Rules (invariants)
//!
//! - **The port speaks this crate's types.** Its methods take the spellings the caller typed and answer the declared
//!   rows or a [`PortRefusal`]; asset-library parses the spellings (a path that is not a library path refuses there,
//!   `item-path-invalid`) and converts its own refusal 1:1. Bus codes are chosen here, never there.
//! - **The port answers agents by id only.** `library.read` names them from one roster read and drops an id the
//!   roster no longer holds.
//! - **Answers are mapped as the bodies built them** — nothing here calls a second body or reads back (ADR-0384).
//! - **A refusal applied nothing** — except `library-config-write-failed`, which leaves the new root live and unsaved
//!   (ADR-0381). **It maps onto a code that does not keep the request id**, except the two `INTERNAL` cases:
//!   `profile-edit-unavailable` and a profile-store write failure (ADR-0379 decisions 3–5).
// ADR-0379
// ADR-0384
// ADR-0386

use std::sync::Arc;

use engram_dashboard_base::path::unquote;
use engram_dashboard_command::{
    blocking_handler, CommandError, CommandTable, ErrorCode, RetryMode,
};
use serde_json::{Map, Value};

use crate::commands::{
    reject_blanks, store_command_error, store_error, store_error_reason, AgentCommandHost,
    AgentRefRow, Blank, LibraryFoldersArgs, LibraryFoldersOk, LibraryKind, LibraryListArgs,
    LibraryListOk, LibraryNewFileArgs, LibraryNewFileKind, LibraryNewFileOk, LibraryNewFolderArgs,
    LibraryNewFolderOk, LibraryReadArgs, LibraryReadOk, LibraryRefreshArgs, LibraryRefreshOk,
    LibraryRelinkArgs, LibraryRelinkOk, LibrarySetRootArgs, LibrarySetRootOk, LibraryStatusArgs,
    LibraryStatusOk, ListRow, RelinkStopRow,
};
use crate::launch::{escape_controls, put_reason, Reason};
use crate::persistence::PROFILES_FILE;
use crate::profile::StoreError;
use crate::types::AgentId;

/// The code a relink `stopped` carries when the agents' rewrite met a profile-store error (`args` = `{why}` — which
/// file did not accept and why, the words a registry-writing command answers with). A store error has no reason code
/// of its own, and `stopped` carries the refusal itself (ADR-0384 decision 8).
// ADR-0386
pub const PROFILE_STORE_REFUSED: &str = "profile-store-refused";

/// The refusal codes a caller can clear and then repeat with the same request — an IO or write failure on a library
/// file, the library root or `library.json` (the share comes back, the lock or permission is released), and no usable
/// library (`library.setRoot` or an IO fix clears it; the daemon's `NoLibrary` is test and harness only): they answer
/// [`RetryMode::AfterCondition`], as an unreadable profile store does (`store_command_error`). Every other refusal
/// keeps its code's default — a retry cannot fix it until the caller changes the request (a stale root generation, a
/// taken name) or the daemon restarts (a read-only `library.json`). asset-library's tests pin its consts to this list.
// ADR-0381
// ADR-0386
pub const RETRY_AFTER_CONDITION: &[&str] = &[
    "library-unavailable",
    "library-write-failed",
    "library-read-failed",
    "library-root-unusable",
    "library-config-write-failed",
];

/// Why a library body refused — the class picks the bus code ([`refusal_error`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortClass {
    Conflict,
    NotFound,
    InvalidArgument,
    /// No library to answer from, or a library file or `library.json` that could not be used — `CONFLICT`.
    Unavailable,
    /// A wiring bug (`profile-edit-unavailable`) — `INTERNAL`, the one class that keeps the request id.
    Internal,
}

/// A refusal crossing the port: nothing was applied, except `library-config-write-failed` (the new root is live but
/// not saved — ADR-0381).
#[derive(Debug)]
pub enum PortRefusal {
    Refused {
        class: PortClass,
        reason: Reason,
    },
    /// The profile store's own error, unchanged — answered as every registry-writing command answers it.
    Store(StoreError),
}

/// One item's detail as the port answers it — [`LibraryReadOk`] with the agents by id.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryReadView {
    pub row: ListRow,
    pub absolute: String,
    pub presets_direct: Vec<String>,
    pub presets_indirect: Vec<String>,
    pub agents_direct: Vec<AgentId>,
    pub agents_indirect: Vec<AgentId>,
    pub excerpt: Option<String>,
    pub settings_fields: Vec<String>,
}

/// **Everything** the `library.*` handlers have the library do (a port) — the real thing = asset-library
/// `LibraryService` (`host.rs`); assemblies without a library plug the daemon's `NoLibrary`.
///
/// ★Synchronous, on a blocking pool thread★: the bodies wait at the library's first-scan entrance and on bounded
/// folder IO, so every handler is a `blocking_handler`.
/// ★Spellings go in as typed★ — the implementation parses them with its own path rules.
// ADR-0012
// ADR-0311
// ADR-0386
pub trait LibraryCommandHost: Send + Sync {
    fn status(&self) -> Result<LibraryStatusOk, PortRefusal>;
    /// `path` = an absolute folder; answers the status after the change.
    fn set_root(&self, path: &str, create: bool) -> Result<LibraryStatusOk, PortRefusal>;
    fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal>;
    /// Refuses only a `folder` that is not a library path — a failed scan is the report.
    fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal>;
    /// `kinds` empty = every kind.
    fn list(
        &self,
        folder: Option<&str>,
        query: Option<&str>,
        kinds: &[LibraryKind],
        builtins: bool,
    ) -> Result<LibraryListOk, PortRefusal>;
    fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal>;
    /// Answers the new folder's path.
    fn new_folder(
        &self,
        parent: Option<&str>,
        name: &str,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal>;
    /// Answers the new file's path.
    fn new_file(
        &self,
        folder: Option<&str>,
        name: &str,
        kind: LibraryNewFileKind,
        root_generation: Option<u64>,
    ) -> Result<String, PortRefusal>;
    fn relink(
        &self,
        from: &str,
        to: &str,
        root_generation: Option<u64>,
    ) -> Result<LibraryRelinkOk, PortRefusal>;
}

/// The return of `library.setRoot` — its fields are the same as [`LibraryStatusOk`]. ★Moved by destructuring
/// (without `..`)★ — if either side gains a field, compilation stops here.
impl From<LibraryStatusOk> for LibrarySetRootOk {
    fn from(status: LibraryStatusOk) -> Self {
        let LibraryStatusOk {
            daemon_id,
            root,
            root_setting,
            state,
            state_why,
            config,
            config_why,
            suggested_new,
            root_generation,
        } = status;
        Self {
            daemon_id,
            root,
            root_setting,
            state,
            state_why,
            config,
            config_why,
            suggested_new,
            root_generation,
        }
    }
}

/// Plugs the nine `library.*` handlers into `table` — the `make_table` line for the library port.
pub(crate) fn insert(
    table: &mut CommandTable,
    host: &Arc<dyn AgentCommandHost>,
    library: Arc<dyn LibraryCommandHost>,
) {
    // The insert rejects are build-decided values (`make_table` doc), so they blow up at assembly.
    let status = Arc::clone(&library);
    table
        .insert(
            "library.status",
            blocking_handler(move |_: LibraryStatusArgs| verb_status(status.as_ref())),
        )
        .expect("could not plug library.status into the table");
    let set_root = Arc::clone(&library);
    table
        .insert(
            "library.setRoot",
            blocking_handler(move |args: LibrarySetRootArgs| {
                verb_set_root(set_root.as_ref(), args)
            }),
        )
        .expect("could not plug library.setRoot into the table");
    let folders = Arc::clone(&library);
    table
        .insert(
            "library.folders",
            blocking_handler(move |_: LibraryFoldersArgs| verb_folders(folders.as_ref())),
        )
        .expect("could not plug library.folders into the table");
    let refresh = Arc::clone(&library);
    table
        .insert(
            "library.refresh",
            blocking_handler(move |args: LibraryRefreshArgs| verb_refresh(refresh.as_ref(), args)),
        )
        .expect("could not plug library.refresh into the table");
    let list = Arc::clone(&library);
    table
        .insert(
            "library.list",
            blocking_handler(move |args: LibraryListArgs| verb_list(list.as_ref(), args)),
        )
        .expect("could not plug library.list into the table");
    let read = (Arc::clone(host), Arc::clone(&library));
    table
        .insert(
            "library.read",
            blocking_handler(move |args: LibraryReadArgs| {
                verb_read(read.0.as_ref(), read.1.as_ref(), args)
            }),
        )
        .expect("could not plug library.read into the table");
    let new_folder = Arc::clone(&library);
    table
        .insert(
            "library.newFolder",
            blocking_handler(move |args: LibraryNewFolderArgs| {
                verb_new_folder(new_folder.as_ref(), args)
            }),
        )
        .expect("could not plug library.newFolder into the table");
    let new_file = Arc::clone(&library);
    table
        .insert(
            "library.newFile",
            blocking_handler(move |args: LibraryNewFileArgs| {
                verb_new_file(new_file.as_ref(), args)
            }),
        )
        .expect("could not plug library.newFile into the table");
    let relink = library;
    table
        .insert(
            "library.relink",
            blocking_handler(move |args: LibraryRelinkArgs| verb_relink(relink.as_ref(), args)),
        )
        .expect("could not plug library.relink into the table");
}

fn verb_status(library: &dyn LibraryCommandHost) -> Result<LibraryStatusOk, CommandError> {
    library
        .status()
        .map_err(|refusal| refusal_error("library.status", refusal))
}

fn verb_set_root(
    library: &dyn LibraryCommandHost,
    args: LibrarySetRootArgs,
) -> Result<LibrarySetRootOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    // One layer of quotes off and nothing else: the host's folder reader takes `\`, and `\` → `/` would turn a verbatim
    //   root (`\\?\D:\lib`, as `library.status` can answer it) into a UNC server named `?`.
    library
        .set_root(unquote(&args.path), args.create.unwrap_or(false))
        .map(LibrarySetRootOk::from)
        .map_err(|refusal| refusal_error("library.setRoot", refusal))
}

fn verb_folders(library: &dyn LibraryCommandHost) -> Result<LibraryFoldersOk, CommandError> {
    library
        .folders()
        .map_err(|refusal| refusal_error("library.folders", refusal))
}

fn verb_refresh(
    library: &dyn LibraryCommandHost,
    args: LibraryRefreshArgs,
) -> Result<LibraryRefreshOk, CommandError> {
    let folder = args.folder.as_deref();
    reject_blanks(&[("folder", folder, Blank::OrOmit)])?;
    library
        .refresh(folder)
        .map_err(|refusal| refusal_error("library.refresh", refusal))
}

fn verb_list(
    library: &dyn LibraryCommandHost,
    args: LibraryListArgs,
) -> Result<LibraryListOk, CommandError> {
    let (folder, query) = (args.folder.as_deref(), args.query.as_deref());
    reject_blanks(&[
        ("folder", folder, Blank::OrOmit),
        ("query", query, Blank::OrOmit),
    ])?;
    library
        .list(
            folder,
            query,
            args.kinds.as_deref().unwrap_or_default(),
            args.builtins.unwrap_or(false),
        )
        .map_err(|refusal| refusal_error("library.list", refusal))
}

fn verb_read(
    host: &dyn AgentCommandHost,
    library: &dyn LibraryCommandHost,
    args: LibraryReadArgs,
) -> Result<LibraryReadOk, CommandError> {
    reject_blanks(&[("path", Some(args.path.as_str()), Blank::NeedsValue)])?;
    let view = library
        .read(&args.path)
        .map_err(|refusal| refusal_error("library.read", refusal))?;
    // One roster read names both lists, so one agent never shows under two names in one answer.
    let roster = host.roster();
    let named = |ids: Vec<AgentId>| -> Vec<AgentRefRow> {
        ids.into_iter()
            .filter_map(|id| {
                roster
                    .iter()
                    .find(|row| row.id == id)
                    .map(|row| AgentRefRow {
                        id: id.to_string(),
                        name: row.canonical_name.clone(),
                    })
            })
            .collect()
    };
    Ok(LibraryReadOk {
        row: view.row,
        absolute: view.absolute,
        presets_direct: view.presets_direct,
        presets_indirect: view.presets_indirect,
        agents_direct: named(view.agents_direct),
        agents_indirect: named(view.agents_indirect),
        excerpt: view.excerpt,
        settings_fields: view.settings_fields,
    })
}

fn verb_new_folder(
    library: &dyn LibraryCommandHost,
    args: LibraryNewFolderArgs,
) -> Result<LibraryNewFolderOk, CommandError> {
    let parent = args.parent.as_deref();
    reject_blanks(&[
        ("parent", parent, Blank::OrOmit),
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
    ])?;
    library
        .new_folder(parent, &args.name, args.root_generation)
        .map(|path| LibraryNewFolderOk { path })
        .map_err(|refusal| refusal_error("library.newFolder", refusal))
}

fn verb_new_file(
    library: &dyn LibraryCommandHost,
    args: LibraryNewFileArgs,
) -> Result<LibraryNewFileOk, CommandError> {
    let folder = args.folder.as_deref();
    reject_blanks(&[
        ("folder", folder, Blank::OrOmit),
        ("name", Some(args.name.as_str()), Blank::NeedsValue),
    ])?;
    library
        .new_file(folder, &args.name, args.kind, args.root_generation)
        .map(|path| LibraryNewFileOk { path })
        .map_err(|refusal| refusal_error("library.newFile", refusal))
}

fn verb_relink(
    library: &dyn LibraryCommandHost,
    args: LibraryRelinkArgs,
) -> Result<LibraryRelinkOk, CommandError> {
    reject_blanks(&[
        ("from", Some(args.from.as_str()), Blank::NeedsValue),
        ("to", Some(args.to.as_str()), Blank::NeedsValue),
    ])?;
    library
        .relink(&args.from, &args.to, args.root_generation)
        .map_err(|refusal| refusal_error("library.relink", refusal))
}

/// The one mapping of a port refusal onto a command error (ADR-0379 decision 3): `Unavailable` → `CONFLICT`, the
/// other classes onto their namesakes; `args` = `{code, args, line?}` of the reason — the keys of one `args.reasons`
/// row; the message names the command and the code. A store error answers as every registry-writing command does,
/// with no args.
// ADR-0379
// ADR-0381
pub(crate) fn refusal_error(command: &'static str, refusal: PortRefusal) -> CommandError {
    match refusal {
        PortRefusal::Refused { class, reason } => {
            let code = match class {
                PortClass::Conflict | PortClass::Unavailable => ErrorCode::Conflict,
                PortClass::NotFound => ErrorCode::NotFound,
                PortClass::InvalidArgument => ErrorCode::InvalidArgument,
                PortClass::Internal => ErrorCode::Internal,
            };
            let mut args = Map::new();
            put_reason(&mut args, &reason);
            let message = refusal_message(command, &reason);
            let error = match RETRY_AFTER_CONDITION.contains(&reason.code.as_str()) {
                true => CommandError::with_retry(code, message, RetryMode::AfterCondition),
                false => CommandError::of(code, message),
            };
            error.with_args(args)
        }
        PortRefusal::Store(error) => store_error(
            command,
            &error,
            store_command_error(
                &error,
                format!(
                    "{command}: {} — nothing was changed",
                    store_error_reason(PROFILES_FILE, &error)
                ),
            ),
        ),
    }
}

/// A relink `stopped` row: the refusal it met, carried as a refusal would carry it — a store error as
/// [`PROFILE_STORE_REFUSED`].
// ADR-0384
// ADR-0386
pub fn relink_stop_row(path: Option<String>, refusal: PortRefusal) -> RelinkStopRow {
    let reason = match refusal {
        PortRefusal::Refused { reason, .. } => reason,
        PortRefusal::Store(error) => Reason {
            code: PROFILE_STORE_REFUSED.to_owned(),
            args: serde_json::json!({ "why": store_error_reason(PROFILES_FILE, &error) }),
            line: None,
        },
    };
    RelinkStopRow {
        path,
        message: refusal_message("library.relink", &reason),
        code: reason.code,
        args: reason.args,
    }
}

/// `<command>: <code>` plus the reason's plain arguments and line — one readable English line; `args` carries them
/// as data.
fn refusal_message(command: &str, reason: &Reason) -> String {
    let mut details: Vec<String> = reason
        .args
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some(format!("{key}: {}", escape_controls(text))),
            Value::Number(number) => Some(format!("{key}: {number}")),
            Value::Bool(flag) => Some(format!("{key}: {flag}")),
            _ => None,
        })
        .collect();
    if let Some(line) = reason.line {
        details.push(format!("line: {line}"));
    }
    match details.is_empty() {
        true => format!("{command}: {}", reason.code),
        false => format!("{command}: {} — {}", reason.code, details.join(", ")),
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`LibraryCommandHost`] — records each call with the values it was given and answers what the test
    //! set (default: every call refuses `library-unavailable`, `refresh` answers an empty report).

    use std::sync::Mutex;

    use super::*;
    use crate::commands::{LibraryStatusOk, ListRow};

    /// One recorded call: the method and its arguments, as a value a test compares.
    pub(crate) type Call = (&'static str, Value);

    #[derive(Default)]
    pub(crate) struct FakeLibrary {
        pub(crate) calls: Mutex<Vec<Call>>,
        /// The next answer of any method — `Ok(value)` is deserialized into that method's answer type.
        pub(crate) next: Mutex<Option<Result<Value, PortRefusal>>>,
    }

    impl FakeLibrary {
        /// A port for tables whose tests do not reach the library.
        pub(crate) fn unplugged() -> Arc<dyn LibraryCommandHost> {
            Arc::new(FakeLibrary::default())
        }

        pub(crate) fn answering(answer: Result<Value, PortRefusal>) -> Arc<Self> {
            let fake = FakeLibrary::default();
            *fake.next.lock().unwrap() = Some(answer);
            Arc::new(fake)
        }

        pub(crate) fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn answer<T: serde::de::DeserializeOwned>(
            &self,
            method: &'static str,
            args: Value,
        ) -> Result<T, PortRefusal> {
            self.calls.lock().unwrap().push((method, args));
            match self.next.lock().unwrap().take() {
                Some(Ok(value)) => {
                    Ok(serde_json::from_value(value).expect("scripted answer shape"))
                }
                Some(Err(refusal)) => Err(refusal),
                None => Err(PortRefusal::Refused {
                    class: PortClass::Unavailable,
                    reason: Reason {
                        code: "library-unavailable".to_owned(),
                        args: serde_json::json!({ "message": "fake" }),
                        line: None,
                    },
                }),
            }
        }
    }

    /// A read view as JSON — [`LibraryReadView`] is not a wire type, so the fake builds it from fields.
    #[derive(serde::Deserialize)]
    struct ReadWire {
        row: ListRow,
        absolute: String,
        presets_direct: Vec<String>,
        presets_indirect: Vec<String>,
        agents_direct: Vec<AgentId>,
        agents_indirect: Vec<AgentId>,
        excerpt: Option<String>,
        settings_fields: Vec<String>,
    }

    impl LibraryCommandHost for FakeLibrary {
        fn status(&self) -> Result<LibraryStatusOk, PortRefusal> {
            self.answer("status", Value::Null)
        }
        fn set_root(&self, path: &str, create: bool) -> Result<LibraryStatusOk, PortRefusal> {
            self.answer(
                "set_root",
                serde_json::json!({ "path": path, "create": create }),
            )
        }
        fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal> {
            self.answer("folders", Value::Null)
        }
        fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal> {
            self.answer("refresh", serde_json::json!({ "folder": folder }))
        }
        fn list(
            &self,
            folder: Option<&str>,
            query: Option<&str>,
            kinds: &[LibraryKind],
            builtins: bool,
        ) -> Result<LibraryListOk, PortRefusal> {
            self.answer(
                "list",
                serde_json::json!({
                    "folder": folder, "query": query, "kinds": kinds, "builtins": builtins,
                }),
            )
        }
        fn read(&self, path: &str) -> Result<LibraryReadView, PortRefusal> {
            let wire: ReadWire = self.answer("read", serde_json::json!({ "path": path }))?;
            Ok(LibraryReadView {
                row: wire.row,
                absolute: wire.absolute,
                presets_direct: wire.presets_direct,
                presets_indirect: wire.presets_indirect,
                agents_direct: wire.agents_direct,
                agents_indirect: wire.agents_indirect,
                excerpt: wire.excerpt,
                settings_fields: wire.settings_fields,
            })
        }
        fn new_folder(
            &self,
            parent: Option<&str>,
            name: &str,
            root_generation: Option<u64>,
        ) -> Result<String, PortRefusal> {
            self.answer(
                "new_folder",
                serde_json::json!({
                    "parent": parent, "name": name, "root_generation": root_generation,
                }),
            )
        }
        fn new_file(
            &self,
            folder: Option<&str>,
            name: &str,
            kind: LibraryNewFileKind,
            root_generation: Option<u64>,
        ) -> Result<String, PortRefusal> {
            self.answer(
                "new_file",
                serde_json::json!({
                    "folder": folder, "name": name, "kind": kind, "root_generation": root_generation,
                }),
            )
        }
        fn relink(
            &self,
            from: &str,
            to: &str,
            root_generation: Option<u64>,
        ) -> Result<LibraryRelinkOk, PortRefusal> {
            self.answer(
                "relink",
                serde_json::json!({ "from": from, "to": to, "root_generation": root_generation }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_command::testing::block_on;
    use engram_dashboard_command::RetryMode;
    use serde_json::json;

    use super::fake::FakeLibrary;
    use super::*;
    use crate::commands::{AgentRosterRow, StartedAgent, COMMAND_SPECS};
    use crate::manager::{Activated, NotStarted, NoticeDelivery, RenameOutcome};
    use crate::profile::{AgentProfile, Refusal, SpawnMode, StoreStatus};
    use crate::queued_input::QueuedListing;
    use crate::types::{CancelError, CancelOutcome, PtyError};

    /// A host that answers only the roster — the one thing a library handler reads from the manager.
    struct Roster(Vec<(AgentId, &'static str)>);

    impl AgentCommandHost for Roster {
        fn roster(&self) -> Vec<AgentRosterRow> {
            self.0
                .iter()
                .map(|(id, name)| AgentRosterRow {
                    id: *id,
                    canonical_name: (*name).to_owned(),
                    cwd: "C:/work".to_owned(),
                    parent: None,
                    live: None,
                })
                .collect()
        }
        fn store_status(&self) -> StoreStatus {
            unreachable!("not a library call")
        }
        fn agent_snapshot(&self, _: AgentId) -> Option<AgentProfile> {
            unreachable!("not a library call")
        }
        fn create_agent(&self, _: AgentProfile) -> Result<AgentProfile, PtyError> {
            unreachable!("not a library call")
        }
        fn activate_profile(
            &self,
            _: &AgentProfile,
            _: SpawnMode,
        ) -> Result<StartedAgent, PtyError> {
            unreachable!("not a library call")
        }
        fn create_and_activate(
            &self,
            _: &AgentProfile,
            _: NoticeDelivery,
        ) -> Result<Activated, NotStarted> {
            unreachable!("not a library call")
        }
        fn rename_agent(&self, _: AgentId, _: Option<String>) -> Result<RenameOutcome, PtyError> {
            unreachable!("not a library call")
        }
        fn reparent_agent(&self, _: AgentId, _: Option<AgentId>) -> Result<bool, PtyError> {
            unreachable!("not a library call")
        }
        fn list_queued_inputs(&self, _: AgentId) -> Option<QueuedListing> {
            unreachable!("not a library call")
        }
        fn cancel_queued_input(&self, _: AgentId, _: &str) -> Result<CancelOutcome, CancelError> {
            unreachable!("not a library call")
        }
        fn interrupt_agent(&self, _: AgentId) -> Result<(), PtyError> {
            unreachable!("not a library call")
        }
    }

    fn table(roster: Vec<(AgentId, &'static str)>, library: Arc<FakeLibrary>) -> CommandTable {
        let mut table = CommandTable::new(COMMAND_SPECS);
        let host: Arc<dyn AgentCommandHost> = Arc::new(Roster(roster));
        insert(&mut table, &host, library);
        table
    }

    fn call(table: &CommandTable, name: &str, mut args: Value) -> Result<Value, CommandError> {
        block_on(table.call(name, &mut args).expect("a name in the table"))
    }

    fn refused(class: PortClass, code: &str, args: Value, line: Option<u32>) -> PortRefusal {
        PortRefusal::Refused {
            class,
            reason: Reason {
                code: code.to_owned(),
                args,
                line,
            },
        }
    }

    /// Every field distinct, so a field carried into another's place fails the comparison.
    fn status_row() -> Value {
        json!({
            "daemon_id": "d1", "root": "C:/lib", "root_setting": "default", "state": "missing",
            "state_why": "the stat timed out", "config": "unsaved", "config_why": "the write failed",
            "suggested_new": "C:/data/library", "root_generation": 41,
        })
    }

    fn read_view(path: &str) -> Value {
        json!({
            "row": list_row(path), "absolute": "C:/lib/x", "presets_direct": [], "presets_indirect": [],
            "agents_direct": [], "agents_indirect": [], "excerpt": null, "settings_fields": [],
        })
    }

    fn list_row(path: &str) -> Value {
        json!({
            "path": path, "kind": "Preset", "name": "p.yaml", "state": null, "references": 2,
            "description": "d",
        })
    }

    /// Every verb hands the port exactly the values the caller typed — spellings unparsed (the host parses them),
    /// omitted options as their stated defaults, enum words in the declared spelling.
    #[test]
    fn each_verb_hands_the_port_the_values_the_caller_typed() {
        let empty_report = json!({
            "changed": 0, "problems": [], "no_kind": [], "skipped_links": [], "errors": [],
        });
        let cases: Vec<(&str, Value, Value, &str, Value)> = vec![
            (
                "library.status",
                json!({}),
                status_row(),
                "status",
                Value::Null,
            ),
            (
                "library.setRoot",
                json!({ "path": "C:/lib" }),
                status_row(),
                "set_root",
                json!({ "path": "C:/lib", "create": false }),
            ),
            (
                "library.setRoot",
                json!({ "path": "C:/lib", "create": true }),
                status_row(),
                "set_root",
                json!({ "path": "C:/lib", "create": true }),
            ),
            (
                "library.setRoot",
                json!({ "path": "\"C:\\data\\lib\"" }),
                status_row(),
                "set_root",
                json!({ "path": "C:\\data\\lib", "create": false }),
            ),
            (
                "library.setRoot",
                json!({ "path": "\\\\?\\D:\\lib." }),
                status_row(),
                "set_root",
                json!({ "path": "\\\\?\\D:\\lib.", "create": false }),
            ),
            (
                "library.folders",
                json!({}),
                json!({ "folders": [] }),
                "folders",
                Value::Null,
            ),
            (
                "library.read",
                json!({ "path": "rules/a.md" }),
                read_view("rules/a.md"),
                "read",
                json!({ "path": "rules/a.md" }),
            ),
            (
                "library.refresh",
                json!({}),
                empty_report.clone(),
                "refresh",
                json!({ "folder": null }),
            ),
            (
                "library.refresh",
                json!({ "folder": "rules" }),
                empty_report,
                "refresh",
                json!({ "folder": "rules" }),
            ),
            (
                "library.list",
                json!({}),
                json!({ "folder": null, "rows": [], "folders": [], "builtins": [] }),
                "list",
                json!({ "folder": null, "query": null, "kinds": [], "builtins": false }),
            ),
            (
                "library.list",
                json!({ "folder": "a", "query": "x", "kinds": ["skillfile", "Preset"], "builtins": true }),
                json!({ "folder": "a", "rows": [], "folders": [], "builtins": [] }),
                "list",
                json!({ "folder": "a", "query": "x", "kinds": ["SkillFile", "Preset"], "builtins": true }),
            ),
            (
                "library.newFolder",
                json!({ "name": "team", "root_generation": 41 }),
                json!("team"),
                "new_folder",
                json!({ "parent": null, "name": "team", "root_generation": 41 }),
            ),
            (
                "library.newFile",
                json!({ "folder": "team", "name": "p", "kind": "Preset" }),
                json!("team/p.yaml"),
                "new_file",
                json!({ "folder": "team", "name": "p", "kind": "Preset", "root_generation": null }),
            ),
            (
                "library.relink",
                json!({ "from": "a.md", "to": "b/a.md", "root_generation": 7 }),
                json!({ "fixed": [], "more": 0, "written": [], "skipped": [], "stopped": null }),
                "relink",
                json!({ "from": "a.md", "to": "b/a.md", "root_generation": 7 }),
            ),
        ];
        for (name, args, answer, method, given) in cases {
            let library = FakeLibrary::answering(Ok(answer));
            let table = table(Vec::new(), Arc::clone(&library));
            call(&table, name, args).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(library.calls(), vec![(method, given)], "{name}");
        }
    }

    /// The created path comes back as `path`.
    #[test]
    fn a_create_answers_the_path_the_port_made() {
        let library = FakeLibrary::answering(Ok(json!("team/p.yaml")));
        let out = call(
            &table(Vec::new(), library),
            "library.newFile",
            json!({ "name": "p", "kind": "Preset" }),
        )
        .expect("create");
        assert_eq!(out, json!({ "path": "team/p.yaml" }));
    }

    /// The port's answer reaches the caller as built — the handler rebuilds nothing (ADR-0384).
    #[test]
    fn answers_are_carried_unchanged() {
        let answer = json!({
            "folder": "a",
            "rows": [list_row("a/p.yaml")],
            "folders": [{ "path": "a/b", "name": "b" }],
            "builtins": [list_row("engram:/default")],
        });
        let library = FakeLibrary::answering(Ok(answer.clone()));
        let out = call(
            &table(Vec::new(), library),
            "library.list",
            json!({ "folder": "a" }),
        )
        .expect("list");
        assert_eq!(out, answer);

        let relinked = json!({
            "fixed": [{ "from": "a.md", "to": "b/a.md" }],
            "more": 1,
            "written": ["p.yaml"],
            "skipped": [{ "path": "q.yaml", "code": "yaml-syntax", "args": { "path": "q.yaml" }, "line": 3 }],
            "stopped": {
                "path": "r.yaml", "code": "library-write-failed",
                "args": { "path": "r.yaml", "why": "x" }, "message": "m",
            },
        });
        let library = FakeLibrary::answering(Ok(relinked.clone()));
        let out = call(
            &table(Vec::new(), library),
            "library.relink",
            json!({ "from": "a.md", "to": "b/a.md" }),
        )
        .expect("relink");
        assert_eq!(out, relinked);
    }

    /// `library.setRoot` answers the very status row the port answered (the two declarations are one shape).
    #[test]
    fn set_root_answers_the_status_row_the_port_answered() {
        let library = FakeLibrary::answering(Ok(status_row()));
        let out = call(
            &table(Vec::new(), library),
            "library.setRoot",
            json!({ "path": "C:/lib" }),
        )
        .expect("set root");
        assert_eq!(out, status_row());
    }

    /// Each class lands on its ADR-0379 code with the reason as `{code, args, line?}` and a message naming the
    /// command and the code; `Unavailable` is `CONFLICT`.
    #[test]
    fn each_refusal_class_maps_onto_its_bus_code_with_the_reason_as_args() {
        let cases = [
            (PortClass::Conflict, ErrorCode::Conflict),
            (PortClass::NotFound, ErrorCode::NotFound),
            (PortClass::InvalidArgument, ErrorCode::InvalidArgument),
            (PortClass::Unavailable, ErrorCode::Conflict),
            (PortClass::Internal, ErrorCode::Internal),
        ];
        for (class, code) in cases {
            let library = FakeLibrary::answering(Err(refused(
                class,
                "some-code",
                json!({ "path": "a.md", "nested": { "x": 1 } }),
                Some(4),
            )));
            let err = call(
                &table(Vec::new(), library),
                "library.read",
                json!({ "path": "a.md" }),
            )
            .expect_err("refused");
            assert_eq!(err.code(), code, "{class:?}");
            assert_eq!(
                Value::Object(err.args().expect("args").clone()),
                json!({ "code": "some-code", "args": { "path": "a.md", "nested": { "x": 1 } }, "line": 4 }),
                "{class:?}"
            );
            assert_eq!(
                err.message(),
                "library.read: some-code — path: a.md, line: 4",
                "{class:?}"
            );
        }
    }

    /// A refusal the caller can clear and then repeat answers `AfterCondition`; any other keeps its code's default.
    #[test]
    fn a_clearable_refusal_answers_retry_after_condition() {
        for code in RETRY_AFTER_CONDITION {
            let library = FakeLibrary::answering(Err(refused(
                PortClass::Unavailable,
                code,
                json!({ "why": "x" }),
                None,
            )));
            let err = call(
                &table(Vec::new(), library),
                "library.relink",
                json!({ "from": "a.md", "to": "b.md" }),
            )
            .expect_err("refused");
            assert_eq!(
                (err.code(), err.retry()),
                (ErrorCode::Conflict, RetryMode::AfterCondition),
                "{code}"
            );
        }
        for code in [
            "library-config-read-only",
            "root-generation-stale",
            "name-exists",
        ] {
            let library =
                FakeLibrary::answering(Err(refused(PortClass::Conflict, code, json!({}), None)));
            let err = call(
                &table(Vec::new(), library),
                "library.newFolder",
                json!({ "name": "x" }),
            )
            .expect_err("refused");
            assert_eq!(err.retry(), ErrorCode::Conflict.default_retry(), "{code}");
        }
    }

    /// A reason with no line carries no `line` key — the shape of an `args.reasons` row.
    #[test]
    fn a_reason_without_a_line_carries_no_line_key() {
        let library = FakeLibrary::answering(Err(refused(
            PortClass::Unavailable,
            "library-unavailable",
            json!({ "message": "no root" }),
            None,
        )));
        let err =
            call(&table(Vec::new(), library), "library.folders", json!({})).expect_err("refused");
        assert_eq!(
            Value::Object(err.args().expect("args").clone()),
            json!({ "code": "library-unavailable", "args": { "message": "no root" } })
        );
        assert_eq!(
            err.message(),
            "library.folders: library-unavailable — message: no root"
        );
    }

    /// A store error answers as every registry-writing command (`store_command_error`): the agent picks the code and
    /// the retry, the text says which file did not accept, and there are no args.
    #[test]
    fn a_store_refusal_answers_as_a_registry_writing_command() {
        let cases = [
            (
                StoreError::Refused(Refusal::Unreadable),
                ErrorCode::Conflict,
                RetryMode::AfterCondition,
            ),
            (
                StoreError::ReadOnly(Refusal::Newer { found: 3 }),
                ErrorCode::Conflict,
                ErrorCode::Conflict.default_retry(),
            ),
            (
                StoreError::Io(std::io::Error::other("disk full")),
                ErrorCode::Internal,
                ErrorCode::Internal.default_retry(),
            ),
        ];
        for (error, code, retry) in cases {
            let reason = store_error_reason(PROFILES_FILE, &error);
            let library = FakeLibrary::answering(Err(PortRefusal::Store(error)));
            let err = call(
                &table(Vec::new(), library),
                "library.relink",
                json!({ "from": "a.md", "to": "b.md" }),
            )
            .expect_err("refused");
            assert_eq!((err.code(), err.retry()), (code, retry), "{reason}");
            assert!(err.args().is_none(), "{reason}");
            assert_eq!(
                err.message(),
                format!("library.relink: {reason} — nothing was changed")
            );
        }
    }

    /// `library.read` names the agents from the roster and drops an id the roster no longer holds.
    #[test]
    fn read_names_the_agents_from_the_roster_and_drops_unknown_ids() {
        let (bob, ann, gone) = (AgentId::new_v4(), AgentId::new_v4(), AgentId::new_v4());
        let view = json!({
            "row": list_row("p.yaml"),
            "absolute": "C:/lib/p.yaml",
            "presets_direct": ["q.yaml"],
            "presets_indirect": ["r.yaml"],
            "agents_direct": [bob, gone],
            "agents_indirect": [ann],
            "excerpt": "e",
            "settings_fields": ["f"],
        });
        let library = FakeLibrary::answering(Ok(view));
        let out = call(
            &table(vec![(bob, "bob"), (ann, "ann")], library),
            "library.read",
            json!({ "path": "p.yaml" }),
        )
        .expect("read");
        assert_eq!(
            out,
            json!({
                "row": list_row("p.yaml"),
                "absolute": "C:/lib/p.yaml",
                "presets_direct": ["q.yaml"],
                "presets_indirect": ["r.yaml"],
                "agents_direct": [{ "id": bob.to_string(), "name": "bob" }],
                "agents_indirect": [{ "id": ann.to_string(), "name": "ann" }],
                "excerpt": "e",
                "settings_fields": ["f"],
            })
        );
    }

    /// A blank value is refused before the port is called — an unset shell variable is not a path.
    #[test]
    fn blank_values_are_refused_before_the_port_is_called() {
        let cases = [
            ("library.setRoot", json!({ "path": " " })),
            ("library.refresh", json!({ "folder": "" })),
            ("library.list", json!({ "query": "  " })),
            ("library.list", json!({ "folder": "" })),
            ("library.read", json!({ "path": "" })),
            ("library.newFolder", json!({ "name": " " })),
            ("library.newFolder", json!({ "parent": "", "name": "x" })),
            ("library.newFile", json!({ "name": "", "kind": "Markdown" })),
            (
                "library.newFile",
                json!({ "folder": " ", "name": "x", "kind": "Markdown" }),
            ),
            ("library.relink", json!({ "from": "", "to": "b.md" })),
            ("library.relink", json!({ "from": "a.md", "to": " " })),
        ];
        for (name, args) in cases {
            let library = Arc::new(FakeLibrary::default());
            let err = call(&table(Vec::new(), Arc::clone(&library)), name, args.clone())
                .expect_err("blank refused");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name} {args}");
            assert!(err.args().is_none(), "{name} {args}");
            assert!(
                library.calls().is_empty(),
                "{name} {args}: the port was called"
            );
        }
    }

    /// A relink stop carries the refusal it met as a refusal would: the reason's code and args and a message naming
    /// the command — and a store error as `profile-store-refused` with the store's words in `why`.
    #[test]
    fn a_relink_stop_carries_the_refusal_it_met() {
        let stop = relink_stop_row(
            Some("r.yaml".to_owned()),
            refused(
                PortClass::Unavailable,
                "library-write-failed",
                json!({ "path": "r.yaml", "why": "denied" }),
                None,
            ),
        );
        assert_eq!(
            stop,
            RelinkStopRow {
                path: Some("r.yaml".to_owned()),
                code: "library-write-failed".to_owned(),
                args: json!({ "path": "r.yaml", "why": "denied" }),
                message: "library.relink: library-write-failed — path: r.yaml, why: denied"
                    .to_owned(),
            }
        );

        let error = StoreError::Refused(Refusal::Unreadable);
        let why = store_error_reason(PROFILES_FILE, &error);
        let stop = relink_stop_row(None, PortRefusal::Store(error));
        assert_eq!(stop.path, None);
        assert_eq!(stop.code, PROFILE_STORE_REFUSED);
        assert_eq!(stop.args, json!({ "why": why }));
        assert_eq!(
            stop.message,
            format!("library.relink: {PROFILE_STORE_REFUSED} — why: {why}")
        );
    }

    /// Control characters in a caller-typed spelling are escaped in the message line.
    #[test]
    fn the_message_escapes_control_characters() {
        let message = refusal_message(
            "library.read",
            &Reason {
                code: "not-found".to_owned(),
                args: json!({ "path": "a\nb" }),
                line: None,
            },
        );
        assert_eq!(message, "library.read: not-found — path: a\\nb");
    }
}
