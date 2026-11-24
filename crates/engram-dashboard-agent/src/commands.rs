//! `agent.*` commands — live next to the code that works with **declaration and body as one set** (`AgentManager`) (ADR-0155).
//!
//! ★This table is the real entry of the control verbs — **two** surfaces★: the CLI's `/control/agent` route (finds `{verb}` as
//!   `agent.<verb>`) and the envelope the client issues on the command bus. Both pass through the daemon's one common entry
//!   (daemon `control/commands.rs`), so this file's return shapes, error codes and notification counts are exactly the response of `engram
//!   agent …` and the answer the LLM receives on the bus.
//! ★Hosted by the daemon, declared by the core★ (ADR-0029 + TRD §2-3).
//!
//! ★`usage.*` (the two usage-limit verbs) are declared here too — their bodies are not★: this crate produces the vocabulary
//!   (`AgentBackend` word · row types), so the declaration lives here (ADR-0155 decision 1) and the daemon's usage service plugs the real
//!   thing in through [`UsageCommandHost`] (TRD S21 usage-limit-slot §3 #18).
//!
//! ★`library.*`, `preset.*` and the agent-preset commands are declared here too — their bodies are asset-library's★: this
//!   crate owns the command vocabulary (ADR-0155 decision 1 · ADR-0311); the handlers and the third port,
//!   [`LibraryCommandHost`], are in `library_commands.rs` (one `declare_commands!` block per module, so the
//!   declarations stay in this block). `agent.new`'s `presets` are judged through that port too.
//!
//! Entry points: [`make_table`](fn.make_table.html) (assembly) · [`AgentCommandHost`](trait.AgentCommandHost.html) (injection seam) ·
//!   [`UsageCommandHost`](trait.UsageCommandHost.html) (usage port) · [`LibraryCommandHost`] (library port).

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_base::path::normalize_spelling;
use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandTable, ErrorCode, RetryMode,
};

use crate::launch::{notice_text, LaunchBlocked, Notice, Reason};
use crate::library_commands::{coded_error, refusal_error, PortClass, PortRefusal};
pub use crate::library_commands::{LibraryCommandHost, PresetSection};
use crate::manager::{
    Activated, AgentManager, NotStarted, NoticeDelivery, RenameOutcome, WorkspaceRefusal,
    WorkspaceSet,
};
use crate::persistence::PROFILES_FILE;
use crate::queued_input::{ListedState, QueuedListing};
// An alias telling the core enum apart from the declaration vocabulary of the same name below.
use crate::profile::AgentOutputFormat as CoreAgentOutputFormat;
use crate::profile::{AgentCommand, AgentProfile, Refusal, SpawnMode, StoreError, StoreStatus};
use crate::types::{
    AgentId, AgentStatus, CancelError, CancelOutcome, PtyError, AGENT_STATE_LIVE,
    AGENT_STATE_SLEEPING, RENAME_OUTCOME_RENAMED, RENAME_OUTCOME_UNCHANGED,
};
use crate::usage::UsageVendorKey;

// ★Success responses are flat (user decision 2026-08-13)★: each command declares its return, so there is no reason to wrap one more
//   layer `{"agent":{…}}`.
declare_commands! {
    // v9 (2026-11-01): the old presets (working-folder bookmarks) were removed — `agent.new` lost its `preset` field
    //   and `cwd` became required (`NOT_FOUND` left its advertised errors), and the `agent.list` answer lost
    //   `store.presets` (TRD S25-preset-library §5-2 1). The same unreleased generation: the `agent.spawn` answer
    //   gains `notices` (TRD S25 step 16), and with `cwd` a launch blocked before the add answers `CONFLICT` with
    //   nothing created (TRD S25 step 20 — the `agent.spawn` summary). Also: `agent.spawn` is declared with the launch
    //   deadline class (ADR-0369 — the schema entry gains `"deadline":"Launch"`). The nine `library.*`, the eight
    //   `preset.*` and the four agent-preset commands come in this generation too, as do `agent.setWorkspace` and
    //   `agent.duplicate`, and `agent.new` gains `presets`, `root_generation` and the answer's `skipped` (ADR-0386).
    //   Library mounts (ADR-0400 decision 20) land in it too: `library.list` lost its `builtins` argument and the
    //   answer's `builtins` rows (the built-in mount `engram` is listed as a mount), and every preset view (`preset.get`
    //   and the preset edits' answers) gains `notices`. Then the mount commands: `library.status` answers `mounts` (its
    //   `root`, `root_setting`, `state` and `state_why` left), `library.addMount` · `removeMount` · `relocateMount` are
    //   added and `library.setRoot` is removed (ADR-0400 decision 7), and `preset.reorder`'s `section` became a
    //   lowercase word (ADR-0400 decision 22). `library.renameMount` is added in it too (ADR-0400 decision 12). So are
    //   the file and trash commands `library.move` · `rename` · `copy` · `delete` · `listTrash` · `restore`, the five
    //   writes with the launch deadline class (ADR-0419), and `library.emptyTrash`, shell only (`SHELL_ONLY`) and with
    //   the launch deadline class too.
    // v8: `agent.resolve` added (ADR-0307 · TRD S28 §6).
    // v7: the answer of `agent.list` gained `store` — the state of the two registry stores (agents · presets) (ADR-0291 R17 · D6).
    // v6: `agent.interrupt` (ADR-0237) · `usage.get`·`usage.refresh` (TRD S21 usage-limit-slot §1-6) added; two branches each raised it to
    //   6 and v6 never shipped, so they merged into one generation.
    // v5: `agent.listQueuedInputs`·`agent.cancelQueuedInput` added (ADR-0231).
    // v4: the `backend` vocabulary of `agent.new` went from `Claude` alone → `Claude`·`Codex` (ADR-0219). No names or fields grew but
    //   **the set of words that field accepts** changed, and the generated output carries it
    //   (`"backend":{"enum":[…]}` of `bindings/commands.schema.json`); without raising it, a build that creates codex and one that refuses
    //   it would report the same generation.
    //   ★The generation is raised when the declaration changes, not only when names grow.★
    // v3: the `backend` of `agent.new` went from optional → required — a breaking change removing the silent claude default (reason: that
    //   field's doc). This number is diagnostic; the receiving side does not refuse by it
    //   (the RegisterCommands branch of `connection_core`).
    catalog_version: 9;

    /// One row of the roster.
    struct AgentRow {
        id: String,
        name: String,
        state: String,
        cwd: String,
        parent: Option<String>,
    }

    /// One line of the `agent.resolve` answer — the agent one requested word pointed at.
    struct ResolvedAgentRow {
        /// The word carried in the request, as is.
        target: String,
        id: String,
        /// The current roster name.
        name: String,
        /// The same decision as `state` being `live` in `agent.list`.
        live: bool,
    }

    // ── Roster store state (`store` of `agent.list` — ADR-0291 R17 · D6) ─────────────────────────────────────
    // ★A macro enum is only a string enumeration, so the store state (the tag enum `profile::StoreStatus` carrying a reason) is spread
    //   into a word + optional fields★ — same shape as the usage rows.

    /// The state of one store file — `Writable` (writable) | `ReadOnly` (the daemon did not accept that file when it started — the roster is
    /// empty, and for this whole run requests changing the roster are `CONFLICT` (`NOT_FOUND` if there is no entry to point at). It clears only by fixing
    /// or replacing the file and then restarting the daemon) |
    /// `Refusing` (the last save was refused — the next change decides again and it clears if accepted. If the file was fixed in the meantime,
    /// this value is stale).
    enum StoreStateWord {
        Writable,
        ReadOnly,
        Refusing,
    }

    /// The reason the store does not accept — `Newer` (a file written by a newer edition than this daemon) | `Unreadable` (the file could not be read — lock ·
    /// permission etc.).
    enum StoreRefusalWord {
        Newer,
        Unreadable,
    }

    /// The state row of one store file.
    struct StoreStatusRow {
        state: StoreStateWord,
        /// The reason for `ReadOnly` · `Refusing` — `null` if `Writable`.
        reason: Option<StoreRefusalWord>,
        /// For `Newer`, the edition number that file wrote — otherwise `null`.
        file_version: Option<u64>,
    }

    /// The roster store — `agents` (the profile file `agent-profiles.json` · agent roster).
    struct RosterStoreRow {
        agents: StoreStatusRow,
    }

    /// The backend to run the new agent on — ★the values accepted today are the two `Claude` and `Codex`★ (codex came in
    /// on 2026-09-22).
    ///
    /// The core has other backends too (gemini · shell of `agent::backend`), but **it has not been confirmed that the profile creation path
    /// runs with them** — so the field is left open but the values are narrowed to the confirmed two (so the contract's **shape** does not
    /// change later).
    /// ★The worth of adding a variant is decided by **whether its pair already exists**★: the run command (`AgentCommand`)
    /// **is written as is into the profile file** with `#[serde(tag = "kind")]`, and an old build meeting an unknown kind treats that
    /// file **as one lump** as damaged and **comes up with an empty roster** — the first save copies that file aside to `.corrupt` and overwrites it
    /// (persistence `FileProfileStore::load` · builds older than that move it to `.corrupt-<ms>` at load). So
    /// adding here a backend **not yet** in `AgentCommand` is a migration that breaks disk compatibility, and adding a backend whose pair **already
    /// exists** is not a migration.
    /// ★`Codex` was the latter★ — `AgentCommand::Codex { extra_args, output_format }` existed before that, and
    /// the human menu (`agentlist.createCodex`·`createCodexJson`) was already writing **exactly that shape** into `agents.json`.
    /// What opened this door was neither a new kind, a new field nor a schema bump — ★at one point this place said 「opening codex is a migration that breaks disk compatibility」,
    /// but the human door was already writing the same shape, so
    /// it was not true even then.★
    /// ★The real case of the former is `Gemini`★ — `agent::backend::gemini` stands as a stub, but `AgentCommand` has
    /// no pair for it. An edit adding that word here **puts a new kind onto disk**, so the migration above must be designed
    /// along with it. Do not read the two cases with the same weight.
    /// ★**Why** it is open and **how the next word is opened** are carried by [`LLM_BACKEND_POLICY`]★ — the other
    /// creation doors look at that table too (that table's header writes the doors out separately).
    enum AgentBackend {
        Claude,
        Codex,
    }

    /// The output format (= render mode) of the new agent — fixed at creation and immutable afterwards (ADR-0044/0078).
    ///
    /// `Terminal` = PTY interactive (xterm render) · `StreamJson` = headless NDJSON stream.
    /// The vocabulary must be the same as the core `profile::AgentOutputFormat` **in both variants and wire notation** — ★since the rename (ADR-0194)
    /// this declaration vocabulary and the core enum **use the same name**★. When pointing at it, write it with the path (the bare name
    /// points at this declaration within this file — that is why the import above puts an alias on the core side). If they diverge, the frontend allowlist
    /// (`coerceOutputFormat`) and this entry read the same word differently. The place where the compiler, not prose, keeps that match
    /// is `tests::the_declared_vocabularies_are_pinned_to_the_core_ones`.
    enum AgentOutputFormat {
        Terminal,
        StreamJson,
    }

    /// The two fields of a pending-cancel entry — the registry's reduced state as is (both are needed to re-reduce later events on top of the snapshot).
    struct QueuedInputCancel {
        /// The response to our cancel request — `none` (none yet) | `not_removed` (it answered it did not remove it, or the request failed).
        answer: String,
        /// The vendor close of that id has already been seen.
        vendor_closed: bool,
    }

    /// One line of the waiting list.
    struct QueuedInputRow {
        /// The input id — used as is as the `input_id` of `agent.cancelQueuedInput`.
        id: String,
        text: String,
        /// `queued` | `sent` (the link handed it over and waits for the echo) | `unconfirmed` (the link handed it over but it is unknown whether it was accepted —
        /// mail waits for that outcome · ★no backend emits it today★ — ADR-0235) | `cancelling` (pending cancel — stays on the list until
        /// the outcome happens).
        state: String,
        /// Carried only on `cancelling` rows — otherwise `null`.
        cancel: Option<QueuedInputCancel>,
    }

    // ── Usage limits (`usage.*` — TRD S21 usage-limit-slot §1-6) ──────────────────────────────────────
    // ★A macro enum is only a string enumeration, so the wire state (a tag enum carrying fields) is spread into a word + optional fields★
    //   — words map 1:1 to the six wire states, fields are carried only in that state (`UsageVendorRow`).

    /// The kind of usage window — `FiveHour` (5 hours) · `Weekly` (weekly) · `ModelWeekly` (per-model weekly — `label` is its name).
    enum UsageWindowWord {
        FiveHour,
        Weekly,
        ModelWeekly,
    }

    /// One usage limit window.
    struct UsageWindowRow {
        window: UsageWindowWord,
        /// Carried only for `ModelWeekly` (the display name the vendor gave) — otherwise `null`.
        label: Option<String>,
        /// Percentage 0–100 of the amount **used** (not the amount left). `null` = unknown — not 0.
        used_pct: Option<f64>,
        /// The amount left — the same `floor(clamp(100 − used_pct, 0, 100))` as the screen. `null` if `used_pct` is `null`.
        left_pct: Option<u64>,
        /// Reset time, epoch seconds — the only absolute time of this row.
        resets_at: Option<u64>,
        /// Seconds elapsed since this value was observed (as of the moment of answering).
        age_secs: u64,
        /// The daemon confirmed the reset has passed — the value is that of the previous window.
        expired: bool,
    }

    /// Whether the answer came from the query this request waited for — a word moving daemon `usage_service::UsageServed` as is.
    enum UsageServedWord {
        /// A value fetched by the successful query this request waited for (or a query after it).
        Fresh,
        /// No new value was received during this request (no query · failure · wait cap exceeded) — it is the value held.
        Cached,
    }

    /// The state of the vendor query — 1:1 with the six wire `UsageVendorState` (the fields are spread by [`UsageVendorRow`]).
    enum UsageStateWord {
        Ready,
        NotInstalled,
        NeedsLogin,
        Unavailable,
        Failed,
        Rejected,
    }

    /// A launch notice (`agent.spawn`) — something the launch left out, on purpose or because it was not found
    /// (`missing-item-skipped`, `library-unavailable-skipped`). Not a refusal: the launch went ahead.
    struct LaunchNoticeRow {
        /// Fixed notice code.
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        /// Line in the file it names, or `null`.
        line: Option<u32>,
        /// The notice as one readable line.
        text: String,
    }

    // ── Library (`library.*` — ADR-0386) ──────────────────────────────────────────────────────────────
    // Every row is typed; `Json` holds only `args`, whose keys depend on `code`. The bodies are asset-library's,
    //   reached through `library_commands::LibraryCommandHost`.

    /// The kind of a library item — `SkillFile` is a skill folder's `SKILL.md`: it stands in for its skill, is listed
    /// inside the skill folder, and `kinds` matches it only by `SkillFile`, never by `Skill`.
    enum LibraryKind {
        Instruction,
        Skill,
        SkillFile,
        Settings,
        Preset,
    }

    /// What `library.newFile` makes — `Markdown` (an instruction, `.md`) | `Settings` | `Preset` (`.yaml`).
    enum LibraryNewFileKind {
        Markdown,
        Settings,
        Preset,
    }

    /// A reason — a fixed code, its arguments and the line in the file it names.
    struct ReasonRow {
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        line: Option<u32>,
    }

    /// One library item.
    struct ListRow {
        /// Library path, starting with its mount's name (a skill: its folder; a built-in: `engram/…`).
        path: String,
        /// `null` = a broken file that claims no preset or settings kind (its reason is in `state`), or the one row of
        /// a folder behind a link.
        kind: Option<LibraryKind>,
        /// A skill's name; a built-in's id; every other item's file name.
        name: String,
        /// Why it does not read — a broken file, a settings field refusal, a path through a link, a built-in whose
        /// files are missing (`builtin-unavailable`). `null` = it reads.
        state: Option<ReasonRow>,
        /// Presets that reach it plus agents that attach it. A built-in counts only the Default slot: the agents whose
        /// Default slot is on for `engram/default`, 0 for every other built-in.
        references: u32,
        /// An instruction's excerpt, a skill's description, a preset's or settings file's `description`; else empty.
        description: String,
    }

    /// A plain subfolder line of `library.list`.
    struct ListFolderRow {
        path: String,
        name: String,
        /// A mount (a row of the library's top): `ok` | `missing` | `unreadable`, as last seen (no fresh check);
        /// `null` = a folder inside a mount. The built-in mount `engram` is `unreadable` when its folder cannot be
        /// read, yet its rows stay as stand-ins and a launch goes on with `builtin-unavailable`.
        state: Option<String>,
        /// Why a mount's `state` is not `ok`.
        state_why: Option<String>,
    }

    /// One folder of `library.folders`.
    struct LibraryFolderRow {
        path: String,
        name: String,
        /// A skill folder (it holds a `SKILL.md`).
        skill: bool,
        /// A mount (a top folder): `ok` | `missing` | `unreadable`, as last seen (no fresh check); `null` = a folder
        /// inside a mount. The built-in mount `engram` is `unreadable` when its folder cannot be read, yet its rows
        /// stay as stand-ins and a launch goes on with `builtin-unavailable`.
        state: Option<String>,
        /// Why a mount's `state` is not `ok`.
        state_why: Option<String>,
    }

    /// A mount a whole-library answer left out because it cannot be read. The built-in mount `engram` is named when its
    /// folder cannot be read, yet its rows stay as stand-ins and a launch goes on with `builtin-unavailable`.
    struct UnavailableMountRow {
        mount: String,
        /// `missing` | `unreadable`.
        state: String,
        why: String,
    }

    /// A reason tied to one library path.
    struct PathReasonRow {
        path: String,
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        line: Option<u32>,
    }

    /// Where a refresh could not read — `path` `null` = the folder asked for could not be scanned at all (in no mount,
    /// in the built-in mount, or a dot or temp name); items already read are kept.
    struct RefreshErrorRow {
        path: Option<String>,
        why: String,
    }

    /// An agent that refers to a library item.
    struct AgentRefRow {
        id: String,
        name: String,
    }

    /// One old → new pair a relink re-pointed.
    struct RelinkPairRow {
        from: String,
        to: String,
    }

    /// Why a relink stopped after it had written something — the refusal it met, as a refusal would carry it.
    struct RelinkStopRow {
        /// The preset file whose read or write failed; `null` = the agents' rewrite failed.
        path: Option<String>,
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        /// The refusal as one readable line.
        message: String,
    }

    // ── Files and trash (`library.move` … `library.restore` — ADR-0419) ──────────────────────────────────────
    // ADR-0409
    // ADR-0419
    // One flat row per given path or trash row, in the order given: `status` = `done` | `skipped` | `conflict` |
    //   `failed`. `done` fills the row's result fields; every other status fills `code` and `args` — the reason as the
    //   service built it (a skip's code from `host.rs`), so a staging a copy left is named inside `args`.

    /// How a move or copy answers a name taken at its destination — the entry there goes into its mount's trash and the
    /// item takes the name | the item stays where it is | the item takes the next free `name (2)`, `name (3)` … name.
    enum LibraryOnConflict {
        Overwrite,
        Skip,
        KeepBoth,
    }

    /// A row of a mount's trash: its mount, its bundle, and its path inside the mount (as `library.listTrash` lists it).
    struct TrashAddressRow {
        mount: String,
        bundle: String,
        path: String,
    }

    /// One given path of a move or rename.
    struct MoveItemRow {
        /// As given.
        path: String,
        /// `done` | `skipped` | `conflict` | `failed`.
        status: String,
        /// `done`: the library path it is at now (a merged folder: the folder it merged into); else `null`.
        to: Option<String>,
        /// `done`: the entries an overwrite moved into their mount's trash, in move order; else empty.
        trashed: Vec<TrashAddressRow>,
        /// `done` across volumes: the trash row its original went to, in its own mount's trash; else `null`.
        original: Option<TrashAddressRow>,
        /// `done`, a merge: the entries left where they were, each with why; else empty.
        left: Vec<PathReasonRow>,
        /// Not `done`: the reason's code (a skip: `inside-given-folder` · `in-place` · `name-exists` ·
        /// `name-taken-during-copy`); `done`: `null`.
        code: Option<String>,
        /// The code's arguments (keys depend on `code`); `null` when `done`.
        args: Json,
    }

    /// One old → new pair a move's references followed. Apply the pairs in order: each maps what the pairs before it
    /// made of a path.
    struct MovePairRow {
        from: String,
        to: String,
        /// Only `from` itself maps, nothing below it (a merge's emptied source folder).
        exact: bool,
    }

    /// A git working tree the written or skipped preset files lie in, once per mount and tree.
    struct GitTreeRow {
        mount: String,
        /// `true` = in a git working tree · `false` = in none · `null` = could not be told.
        git: Option<bool>,
        /// The folder holding its `.git` when `git` is `true`; else `null`.
        top: Option<String>,
    }

    /// One given path of a copy.
    struct CopyItemRow {
        /// As given.
        path: String,
        /// `done` | `skipped` | `conflict` | `failed`.
        status: String,
        /// `done`: the library path of the copy; else `null`.
        to: Option<String>,
        /// `done`: the entry an overwrite moved into its mount's trash (none or one); else empty.
        trashed: Vec<TrashAddressRow>,
        /// Not `done`: the reason's code (a skip: `inside-given-folder` · `name-exists` · `name-taken-during-copy`);
        /// `done`: `null`.
        code: Option<String>,
        /// The code's arguments (keys depend on `code`); `null` when `done`.
        args: Json,
    }

    /// One given path of a delete.
    struct DeleteItemRow {
        /// As given.
        path: String,
        /// `done` | `skipped` | `failed`.
        status: String,
        /// `done`: the trash row it went to; else `null`.
        trashed: Option<TrashAddressRow>,
        /// Not `done`: the reason's code (a skip: `inside-given-folder`); `done`: `null`.
        code: Option<String>,
        /// The code's arguments (keys depend on `code`); `null` when `done`.
        args: Json,
    }

    /// One given trash row of a restore.
    struct RestoreItemRow {
        mount: String,
        bundle: String,
        path: String,
        /// `done` | `conflict` | `failed`.
        status: String,
        /// `done`: the library path it is back at (with `keep_both`, maybe `name (2)` …); else `null`.
        to: Option<String>,
        /// Not `done`: the reason's code; `done`: `null`.
        code: Option<String>,
        /// The code's arguments (keys depend on `code`); `null` when `done`.
        args: Json,
    }

    /// Why part of a mount's trash was not listed or not touched — a reason code and its arguments.
    struct TrashProblemRow {
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
    }

    /// One row of a trash bundle.
    struct TrashViewRow {
        /// Its path inside the bundle — the item's path in its mount.
        path: String,
        /// The library path a restore puts it back at.
        original: String,
        /// `root` (a deleted item the bundle's record names — a folder is one row) | `found` (a file, link or empty
        /// folder of a bundle with no readable record) | `other` (something else left in a bundle with a record).
        kind: String,
        /// `present` | `through_link` (a link lies on the way — left untouched) | `failed` (a check on the way
        /// failed — `why`).
        state: String,
        /// `present`: `file` | `folder` | `link`; else `null`.
        entry: Option<String>,
        /// `failed`: why; else `null`.
        why: Option<String>,
        /// A staging a failed or partial copy left (or a found temp name) — shown, never restorable.
        staging: bool,
        /// The library path a staging was meant for, when the record names it; else `null`.
        intended: Option<String>,
        /// `library.restore` can take it: not `other`, not a staging, `present`, and `original` holds no dot or
        /// temp name.
        restorable: bool,
    }

    /// One bundle of a mount's trash — what one operation deleted there.
    struct TrashBundleRow {
        bundle: String,
        /// When it was deleted, RFC 3339 UTC (`2026-11-23T08:15:00Z`).
        deleted_at: String,
        /// How its rows were found: `record` | `no_record` (its files and empty folders) | `record_unreadable` (the
        /// same; `why`) | `unchecked` (the bundle could not be checked — no rows; `trouble`).
        source: String,
        /// `record_unreadable`: why; else `null`.
        why: Option<String>,
        /// Why some rows may be missing (a listing inside the bundle failed), or why an `unchecked` bundle has none.
        trouble: Option<String>,
        /// By path.
        rows: Vec<TrashViewRow>,
    }

    /// One mount's trash.
    struct MountTrashRow {
        mount: String,
        /// Why its trash could not be listed (`bundles` is then empty): `trash-unusable` `{mount, why}` (its `.trash`
        /// is a link or not a folder) or `library-unavailable` `{mount, message}` (not read in time, or held as
        /// overlapping another mount); `null` = listed.
        problem: Option<TrashProblemRow>,
        /// By id (oldest first).
        bundles: Vec<TrashBundleRow>,
    }

    /// An empty-all address: one bundle of the trash view with the rows the view listed for it — a row it did not list
    /// stays.
    struct ListedBundleArg {
        mount: String,
        bundle: String,
        /// The rows' paths inside the bundle, as the view listed them.
        rows: Vec<String>,
    }

    /// One addressed trash row of an emptying.
    struct EmptiedRow {
        mount: String,
        bundle: String,
        path: String,
        /// `gone` (it was there and is no longer) | `remaining` (it is still there — a folder row may be partly
        /// emptied) | `not_found` (it was not there, or it is no row: a foreign bundle, a mount not registered, a path
        /// that is no row of the bundle).
        outcome: String,
        /// Not `gone`: why — `remaining`: `library-write-failed` (the OS error code in `args.os_error`),
        /// `path-through-link`, `inside-root`, `library-read-failed`, or why its bundle was not touched; `not_found`:
        /// `not-found`. `gone`: `null`.
        code: Option<String>,
        /// The code's arguments (keys depend on `code`); `null` when `gone`.
        args: Json,
    }

    /// One addressed bundle of an emptying.
    struct EmptiedBundleRow {
        mount: String,
        bundle: String,
        /// `gone` (its folder no longer exists) | `remaining` (its folder stays: rows that stayed, roots the view did not
        /// list, the record) | `not_found` (it was no bundle).
        outcome: String,
        /// Why nothing of it was touched: `library-unavailable` (its mount did not answer or went), `trash-unusable` or
        /// `library-read-failed`; `null` = it was handled.
        untouched: Option<TrashProblemRow>,
    }

    // ── Preset (`preset.*` — ADR-0386) ────────────────────────────────────────────────────────────────

    /// One entry of a preset list as `preset.save` / `preset.check` take it.
    struct PresetEntryArg {
        /// A library path or a built-in (`engram/…`).
        path: String,
        /// The backend word the entry is for (instructions and skills only); `null` = every backend.
        backend: Option<String>,
    }

    /// One entry of a preset list as the file holds it, in file order (a duplicate included).
    struct PresetEntryRow {
        path: String,
        /// `null` = every backend.
        backend: Option<String>,
        /// Its line in the file.
        line: u32,
        /// Why the entry does not read — `not-found` (also for an item in a mount that is not there),
        /// `library-unavailable` `{mount, message}` (its mount cannot be read — the item was not looked up),
        /// `path-through-link`, the item's broken reason or a settings file's first refusal, `wrong-kind`,
        /// `builtin-in-library-preset`, `preset-cycle`; `null` = it reads. What an inner preset reaches is in
        /// `problems`.
        state: Option<ReasonRow>,
    }

    // ADR-0341
    /// One folder entry of a preset — an absolute folder the backend loads natively (not a library item).
    struct FolderRow {
        /// The normalized path; a relative entry (a line problem) as the file spells it.
        path: String,
        /// Its line in the file; `null` = a folder of a `preset.check` draft.
        line: Option<u32>,
        /// `ok` | `missing` | `not_a_directory` | `unreadable`; `null` = not checked (a relative entry, or the library
        /// was unavailable).
        status: Option<String>,
        /// Why it is `unreadable` (the OS error, a timeout or a share not reachable); else `null`.
        status_why: Option<String>,
    }

    /// One item of a conflict and the presets it came through.
    struct ProblemItemRow {
        item: String,
        via: Vec<String>,
    }

    /// A problem of a preset's launch materials, with the absent keys as `null`. A launch leaves out the `missing`
    /// rows (and the `library_unavailable` row of a library mount that is not registered or not there) and refuses on
    /// the others — the rows `agent.spawn` puts in `args.reasons`. A `library_unavailable` row stands for the items of
    /// one mount: `args` = `{mount, message}`.
    // ADR-0387
    // ADR-0400 decision 19
    struct ProblemRow {
        /// `missing` | `broken` | `conflict` | `library_unavailable`.
        kind: String,
        /// The item (`missing` · `broken`); else `null`.
        item: Option<String>,
        /// The presets it came through, outermost first (`missing` · `broken`); else `null`.
        via: Option<Vec<String>>,
        /// Every item a conflict hits (`conflict`); else `null`.
        items: Option<Vec<ProblemItemRow>>,
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        line: Option<u32>,
    }

    /// A preset as `preset.get` shows it — every preset edit answers it after the change.
    struct PresetView {
        path: String,
        /// The preset file's edition — pass it as `base_rev` to `preset.reorder` / `preset.save`.
        rev: String,
        description: Option<String>,
        presets: Vec<PresetEntryRow>,
        instructions: Vec<PresetEntryRow>,
        skills: Vec<PresetEntryRow>,
        settings: Vec<PresetEntryRow>,
        /// The folder entries, normalized and deduplicated (first place kept).
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        /// Every other problem, inner presets' included.
        problems: Vec<ProblemRow>,
        /// What resolving it on its own tells — a built-in it reaches (itself included) that could not be read and
        /// stands in empty (`builtin-unavailable`).
        notices: Vec<LaunchNoticeRow>,
        /// How many agents attach it (`engram/default`: the agents whose default slot is on).
        agents: u32,
    }

    /// An item or folder an edit left out, and why.
    struct SkippedRow {
        /// As given.
        item: String,
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        line: Option<u32>,
    }

    /// One mount of `library.status`.
    struct MountStatusRow {
        name: String,
        /// The folder it maps to (the built-in `engram`: the install's built-in folder, empty when there is none).
        path: String,
        /// Its folder is the default place (saved as `default`).
        default: bool,
        /// The built-in read-only mount `engram`.
        builtin: bool,
        /// `ok` | `missing` (not there or not a folder — its items read as absent) | `unreadable` (there but cannot be
        /// read, did not answer, or overlaps an earlier mount — a launch reaching it is refused); a user mount's is
        /// checked now, the built-in's is the one read at start.
        state: String,
        /// Why `state` is not `ok`.
        state_why: Option<String>,
    }

    /// What points at a mount's name from outside it: references in the preset files of the other mounts (each listed
    /// entry counts) and agents with an attachment in it.
    struct PointingRow {
        presets: u32,
        agents: u32,
    }

    /// What a backend loads a folder file as — `SkillRoot` = a folder it walks for skills by itself.
    enum FolderLoadKindWord {
        Instructions,
        Rule,
        Skill,
        Agent,
        Command,
        SkillRoot,
    }

    /// One file (or skill root) of a folder preview.
    struct PreviewItemRow {
        /// Relative to the folder, `/`-separated.
        path: String,
        kind: FolderLoadKindWord,
    }

    /// An entry the backend may load whose answer could not be predicted.
    struct PreviewUnsureRow {
        /// Relative to the folder, `/`-separated.
        path: String,
        /// `null` = the entry could hold items of several kinds.
        kind: Option<FolderLoadKindWord>,
        /// `unreadable` | `front_matter` | `link` | `dashes_in_front_matter` | `unclear_paths` | `empty_body` |
        /// `hook_keys` | `skill_file_in_command_folder`.
        why: String,
        /// The read error (`unreadable`) or the parser's message (`front_matter`); else `null`.
        message: Option<String>,
        /// The front matter's line in the file (`front_matter`); else `null`.
        line: Option<u32>,
    }

    /// The name a backend gives a loaded skill.
    struct PreviewSkillNameRow {
        /// The skill's `SKILL.md`, relative to the folder.
        path: String,
        name: String,
    }

    /// What one backend in one mode loads from a folder.
    struct PreviewSetRow {
        loads: Vec<PreviewItemRow>,
        /// What another mode of the same backend loads and this one does not.
        omitted: Vec<PreviewItemRow>,
        /// The rows of `loads` the launch carries itself (the rest the backend reads from the folder).
        carried: Vec<PreviewItemRow>,
        unsure: Vec<PreviewUnsureRow>,
        /// Skills that fail the backend's format check — the backend skips them.
        malformed: Vec<PathReasonRow>,
        skill_names: Vec<PreviewSkillNameRow>,
    }

    /// One backend and mode of a folder preview.
    struct PreviewLoadRow {
        backend: String,
        mode: AgentOutputFormat,
        /// The backend reads the folder itself at launch, so the set is a prediction.
        predicted: bool,
        set: PreviewSetRow,
        /// Why a file read got no answer (e.g. `timeout`, budget spent) — the reason only, not the file; the set may
        /// lack what that file decides.
        unread: Option<String>,
    }

    /// Why `preset.add` would refuse the folder.
    struct PreviewRefusalRow {
        /// As given.
        path: String,
        /// `folder-path-relative` | `folder-path-cli-unsafe` | `folder-not-found` | `folder-unreadable`.
        code: String,
        /// For `folder-not-found`: missing or not a folder. For `folder-unreadable`: the OS error, a timeout or a share
        /// not reachable.
        why: Option<String>,
    }

    // ── Agent presets (`agent.presets` · `attachPreset` · `detachPreset` · `reorderPresets` — ADR-0386) ─────────────

    // ADR-0341
    /// A folder entry an agent's presets reach, and the presets it came through.
    struct AgentFolderRow {
        /// The normalized absolute path.
        path: String,
        /// The presets it came through, outermost first.
        via: Vec<String>,
        /// Its line in the last preset of `via`.
        line: u32,
        /// `ok` | `missing` | `not_a_directory` | `unreadable`; `null` = not checked.
        status: Option<String>,
        /// Why it is `unreadable` (the OS error, a timeout or a share not reachable); else `null`.
        status_why: Option<String>,
    }

    /// An agent's presets as `agent.presets` shows them — every agent-preset edit answers it after the change.
    struct AgentPresetsView {
        agent_id: String,
        name: String,
        /// The Default slot (`engram/default`) is on.
        default_preset: bool,
        /// The attached library presets as the agent stores them, in attach order.
        presets: Vec<String>,
        cwd: String,
        /// Why the working folder cannot be changed now — `running` | `preparing` | `conversed`; `null` = it can.
        workspace_lock: Option<String>,
        backend: String,
        mode: AgentOutputFormat,
        /// The attachments' edition — pass it as `base_rev` to `agent.reorderPresets`.
        base_rev: String,
        /// `false` = its backend takes no preset (no settings check): the rows below stay empty and a launch ignores
        /// the presets.
        takes_presets: bool,
        /// The folder entries its presets reach, in expansion order.
        folders: Vec<AgentFolderRow>,
        conflicts: Vec<ProblemRow>,
        /// Every other problem, inner presets' included. A launch leaves out the `missing` rows (and the
        /// `library_unavailable` row of a library mount that is not registered or not there) and refuses on the others.
        problems: Vec<ProblemRow>,
        /// What a launch would leave out on purpose. What it leaves out because it was not found is in `problems`.
        notices: Vec<LaunchNoticeRow>,
    }

    // errors lists only codes **specific to this command** — argument rejection (INVALID_ARGUMENT) and internal failure (INTERNAL)
    //   are emitted by the table, so `CommandSpec::advertised_errors` adds them automatically.

    /// The whole roster — name · liveness · working folder · parent — and the state of the roster store (`store`).
    #[effect(Read)]
    #[since(1)]
    "agent.list" => args AgentListArgs {}
                 -> ok   AgentListOk { agents: Vec<AgentRow>, store: RosterStoreRow }
                 errors [];

    // ADR-0307
    // ★The head `targets[i]: ` of the failure text is a contract★ — the shell (TRD S28 §5-1) traces a failure back to its own argument
    //   name by this head.
    //   The shell does not depend on this crate, so compilation does not stop if it changes. The place building the head =
    //   `target_failure` · the pinning test =
    //   `tests::the_resolve_failure_prefix_is_a_stable_contract`.
    /// Resolves 1–8 words pointing at agents (name or id) at once — `agents` is in the same order as the request. The resolving rules are the same as other verbs
    /// taking `target` (exact id match first · then exact name match — case-sensitive · asleep agents included).
    /// `live` = `state` of `agent.list` is `live`. If even one cannot be resolved, it returns no line and fails. The text of a failure caused by one word
    /// always starts with `targets[i]: ` (i = that word's position counting from 0) — INVALID_ARGUMENT = an empty word (only whitespace) ·
    /// NOT_FOUND = no such agent (the trailing `candidates` of the text = names worth choosing, in name order · `none` if the roster is empty) ·
    /// CONFLICT = that name exists two or more times (the trailing `candidates` of the text = those ids — call again by id). If the count is outside 1–8,
    /// it is INVALID_ARGUMENT without that head.
    #[effect(Read)]
    #[since(8)]
    "agent.resolve" => args AgentResolveArgs {
        targets: Vec<String>,
    } -> ok AgentResolveOk {
        agents: Vec<ResolvedAgentRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Starts an agent: wakes an existing one (`target`) or creates one in `cwd` and starts it.
    /// A preset item that is not found, or the items of a library mount that is not registered or not there, do not
    /// block: they are left out and named in `notices` (one notice per mount, with `mount`). A blocked launch starts
    /// nothing and is `CONFLICT` with `args.reasons`, one row per reason by `kind`: `broken` {kind, item, via, code,
    /// args, line?} (a folder entry that cannot be read included) · `conflict` {kind, items: [{item, via}], code, args,
    /// line?} · `library_unavailable` {kind, code, args: {mount, message}, line?} (a library mount that exists but
    /// cannot be read). Fix the listed items, then start again.
    /// With `cwd`, a launch blocked before the agent is added answers `CONFLICT` with `args.reasons` and creates
    /// nothing; a start failure after the add is `INTERNAL` with `args.agent_id` (plus `args.reasons` when blocked) —
    /// start that agent by target, do not create another.
    // ADR-0369
    // ADR-0387
    #[effect(Write)]
    #[since(1)]
    #[deadline(Launch)]
    "agent.spawn" => args AgentSpawnArgs {
        /// The target to wake (name or id). Mutually exclusive with cwd.
        target: Option<String>,
        /// The working folder to create and launch anew. Mutually exclusive with target.
        cwd: Option<String>,
        /// Used only when creating anew — does not apply to waking.
        name: Option<String>,
    } -> ok AgentSpawnOk {
        agent_id: String,
        name: String,
        state: String,
        created: bool,
        /// Launch notices, each set shown once per agent — empty when nothing was launched or this agent was
        /// already shown the same set.
        notices: Vec<LaunchNoticeRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Registers a new agent in the roster (does not launch it — it stays asleep). `presets` are judged as
    /// `agent.attachPreset` judges them: the agent is created with what can be attached, each one left out is in
    /// `skipped` with why. CONFLICT `library-unavailable` (a library preset named while the library cannot be read —
    /// with `mount` when that preset's mount cannot be read) or `root-generation-stale` creates nothing.
    // ADR-0386
    #[effect(Write)]
    #[since(1)]
    "agent.new" => args AgentNewArgs {
        cwd: String,
        name: Option<String>,
        /// Unspecified = StreamJson.
        output_format: Option<AgentOutputFormat>,
        /// Which backend to run — ★**required. Unspecified is rejected**★ (2026-09-08).
        ///
        /// Unspecified used to mean `Claude`. Since there came to be two backends, that silent default folds **not choosing**
        /// and **choosing claude** into the same value — it is the shape already removed from the spawn packet for the same reason
        /// (the `AgentBackendKind` doc of `protocol` · the daemon's `MISSING_BACKEND`), and leaving only this door
        /// would split that decision per entry.
        backend: AgentBackend,
        /// Library presets to attach — the Default slot (`engram/default`) is on for every new agent. Unspecified = the
        /// Default slot only.
        presets: Option<Vec<String>>,
        /// With `presets` only — the value `library.status` answered; refuses if a mount changed since (checked when
        /// the presets are judged, just before the create). Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok AgentNewOk {
        agent_id: String,
        name: String,
        state: String,
        /// The `presets` not attached, and why; empty without `presets`.
        skipped: Vec<SkippedRow>,
    } errors [CONFLICT];

    /// Changes the display name.
    #[effect(Write)]
    #[since(1)]
    "agent.rename" => args AgentRenameArgs {
        target: String,
        name: String,
    } -> ok AgentRenameOk {
        agent_id: String,
        /// The settled name — may differ from the requested name (roster uniqueness suffix).
        name: String,
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Changes the parent in the tree.
    #[effect(Write)]
    #[since(1)]
    "agent.move" => args AgentMoveArgs {
        target: String,
        /// The new parent (name or id). ★`null` = detach to top level · **an absent field is rejected**★ — so that a single typo field
        /// does not silently run as a hierarchy release, absence and `null` are told apart (the outer Option = whether it was carried).
        parent: Option<Option<String>>,
    } -> ok AgentMoveOk {
        agent_id: String,
        name: String,
        parent: Option<String>,
    } errors [NOT_FOUND, CONFLICT];

    /// Changes an agent's working folder. `cwd` must be an existing absolute folder, else INVALID_ARGUMENT
    /// `workspace-not-found` {cwd, why} — `why` also says `not absolute`, or that the folder did not answer in time
    /// (`timeout`), could not be read, or is on a network share that is not reachable (`share not reachable`). Only an
    /// asleep agent that has never conversed can change it: CONFLICT `workspace-locked` {reason: `running` |
    /// `preparing` | `conversed` | `changing`}. The agent keeps its name.
    // ADR-0386
    #[effect(Write)]
    #[since(9)]
    "agent.setWorkspace" => args AgentSetWorkspaceArgs {
        target: String,
        cwd: String,
    } -> ok AgentSetWorkspaceOk {
        agent_id: String,
        name: String,
        /// The folder as stored.
        cwd: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Registers a copy of an agent beside it (the same parent), asleep, named after it with the next number
    /// (`bob` → `bob(1)`): the same backend, mode, folder, env and presets, with no conversation or history.
    /// INVALID_ARGUMENT when this surface does not create the original's backend; CONFLICT at the roster ceiling.
    /// After a timeout or an unknown outcome, check `agent.list` before calling again.
    // ADR-0386
    #[effect(Write)]
    #[since(9)]
    "agent.duplicate" => args AgentDuplicateArgs {
        target: String,
    } -> ok AgentDuplicateOk {
        agent_id: String,
        name: String,
        state: String,
    } errors [NOT_FOUND, CONFLICT];

    /// The list of inputs a live agent received mid-turn without confirmation yet that they were accepted (non-terminal only). `state` = `queued` (before confirmation —
    /// we hold it or handed it to the agent and wait for the outcome. codex hands it over by steer when the running turn's tool ends, or to the next turn if the turn
    /// ends without a tool · claude hands it to the CLI queue right away) | `sent` (codex handed it to the agent and waits for the echo saying it was received
    /// — cancelling does not remove it) | `unconfirmed` (handed over but unknown whether it was accepted —
    /// it stays on the list and mail waits for that outcome, and cancelling removes it right away · ★no backend emits it today★ — that stage of codex
    /// was removed, ADR-0235) | `cancelling` (a cancel was requested — stays until the agent produces an outcome).
    /// `cancel.answer` = `none` (no answer yet) | `not_removed` (the agent answered it could not remove it, or the request failed) ·
    /// `cancel.vendor_closed` = the agent already closed that entry.
    /// `stopped_after_error` = the previous turn ended with a real error and no turn has ended successfully since — meanwhile mail
    /// does not go in automatically (kernel `last_end_failed`). User text codex holds may go out again the moment new user text comes in
    /// (ADR-0235 decision 4). An asleep agent holds no list (NOT_FOUND).
    #[effect(Read)]
    #[since(5)]
    "agent.listQueuedInputs" => args AgentListQueuedInputsArgs {
        target: String,
    } -> ok AgentListQueuedInputsOk {
        inputs: Vec<QueuedInputRow>,
        /// The seq of the last list event the rows reduced (`null` = none yet). Compared only within `epoch` — for reattach comparison.
        as_of_seq: Option<u64>,
        /// Incarnation marker — compared only for match/mismatch.
        epoch: u32,
        stopped_after_error: bool,
    } errors [NOT_FOUND, CONFLICT];

    /// Cancels one entry of the waiting list (the same command as the ✕ on screen) — `outcome` = `cancelled` (withdrawn right away) |
    /// `requested` (a cancel was requested — the list shows the outcome). An id not on the list is NOT_FOUND. CONFLICT = the name is
    /// ambiguous (two or more with the same name — point at it by id) or a connected viewer holds this agent's input (try again after it lets go).
    #[effect(Write)]
    #[since(5)]
    "agent.cancelQueuedInput" => args AgentCancelQueuedInputArgs {
        target: String,
        /// ★Required★ — no default like 「the most recent if absent」 (TRD §5-6).
        input_id: String,
    } -> ok AgentCancelQueuedInputOk {
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Interrupts the running turn (≠ kill — the process lives). `outcome` = `requested` (the interrupt was sent — whether the turn actually stopped is announced by the turn-end event).
    /// NOT_FOUND = no such agent or it is asleep. CONFLICT = the name is ambiguous · there is no turn to interrupt · this agent
    /// does not support interrupt · a connected viewer holds this agent's input.
    #[effect(Write)]
    #[since(6)]
    "agent.interrupt" => args AgentInterruptArgs {
        target: String,
    } -> ok AgentInterruptOk {
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// The usage limits of one backend (5-hour · weekly · per-model weekly) — the values the daemon holds. If the next automatic query time has passed,
    /// it launches a query here (or attaches to one in progress) and waits up to 5 seconds for it to end — past that it answers with the held value with
    /// `in_flight` = true and the query continues. `served` = `Fresh` (a value fetched by the query this request waited for) |
    /// `Cached`. `state` = `Ready` | `NotInstalled` | `NeedsLogin` | `Unavailable` (this account has no limit info — no windows
    /// are carried) | `Failed` (`next_attempt_in_secs` = until the next automatic query) | `Rejected` (upstream refused —
    /// until `retry_in_secs` passes, even `usage.refresh` does not query). Even on failure, the windows are the last known values.
    /// The cause of an abnormal state = `detail_kind` (category word) · `detail_code` (the number upstream gave) · `upstream` (upstream
    /// original text — secrets masked · 200 chars). A window's `left_pct` = the amount left (`used_pct` is the amount used), and `null` means unknown (not
    /// 0). Time fields are relative seconds as of the moment of answering, and only `resets_at` is epoch seconds. A backend whose field this daemon does not hold is
    /// NOT_FOUND.
    #[effect(Read)]
    #[since(6)]
    "usage.get" => args UsageGetArgs {
        /// Whose backend's limits — the same word as `agent.new`.
        backend: AgentBackend,
    } -> ok UsageVendorRow {
        backend: AgentBackend,
        /// The account within the same backend — always `"default"` for now (the daemon's default login).
        account_key: String,
        plan: Option<String>,
        /// Only the windows the vendor gave — in the order 5-hour · weekly · per-model weekly. Windows with unknown utilization (`used_pct: null`) are carried too.
        windows: Vec<UsageWindowRow>,
        /// The query is still in progress — the values of this row are from before that query.
        in_flight: bool,
        served: UsageServedWord,
        state: UsageStateWord,
        /// Carried only for `Failed` — seconds left until the next automatic query (0 if already passed).
        next_attempt_in_secs: Option<u64>,
        /// Carried only for `Rejected` — until these seconds pass, even a forced refresh sends no query.
        retry_in_secs: Option<u64>,
        /// The category word of an abnormal state (e.g. `timeout`·`rpc_error`) — an untranslated string. `null` = the cause is unknown.
        detail_kind: Option<String>,
        /// The number upstream gave (Codex JSON-RPC `error.code`).
        detail_code: Option<i64>,
        /// The upstream original text (secrets masked · whitespace-class controls become spaces · 200 chars) — it goes only to the UI and this row, not to logs.
        ///
        /// ★The `Debug` of this type prints the original as is — do not put it into logs with `{:?}` or a tracing `?` field, whether the whole row or this
        ///   field★. The declaration macro always derives `Debug` and there is no door to turn it off per type, so it cannot be hand-written like the masked `Debug` (agent
        ///   `usage::UpstreamText` · protocol `UsageStateDetail`). The way to the log is the one result log after the deadline,
        ///   and there the key of this name is stripped from the JSON copy (daemon `command_delivery::log_late_local`).
        upstream: Option<String>,
    } errors [NOT_FOUND];

    /// Forces a new query of the usage limits (the same command as ⟳ on screen) — launches a query (or attaches to one in progress) and waits up to 5 seconds for
    /// it to end, answering with the same row as `usage.get`. There is no call interval limit — it queries again even if the previous query just ended,
    /// and if frequent, upstream may refuse (that refusal may show only as `Failed`, not `Rejected`). Answering with the held value (`Cached`)
    /// without querying = the upstream refusal period remains (even if a value came in during the refusal and `state` is
    /// `Ready`, the period remains — `retry_in_secs` is not carried then). Even if the query fails or does not end within 5 seconds, it is
    /// `Cached` (the query did go out then). The meaning of the fields is the same as `usage.get`. A backend whose field this daemon does not hold is NOT_FOUND.
    #[effect(Write)]
    #[since(6)]
    "usage.refresh" => args UsageRefreshArgs {
        /// Whose backend's limits — the same word as `agent.new`.
        backend: AgentBackend,
    } -> ok UsageRefreshOk {
        // ★The fields must not differ by a character from `UsageVendorRow`★ — the macro builds a new return struct per command, so two
        //   commands cannot share a type. The pairing is `From<UsageVendorRow>` (destructuring — compilation stops on
        //   divergence) and the test comparing the two return schemas' bytes. Field meanings: the `UsageVendorRow` doc.
        backend: AgentBackend,
        account_key: String,
        plan: Option<String>,
        windows: Vec<UsageWindowRow>,
        in_flight: bool,
        served: UsageServedWord,
        state: UsageStateWord,
        next_attempt_in_secs: Option<u64>,
        retry_in_secs: Option<u64>,
        detail_kind: Option<String>,
        detail_code: Option<i64>,
        /// Upstream original text — ★`Debug` prints the original; do not put it into logs with `{:?}`★ (the doc of `UsageVendorRow.upstream`).
        upstream: Option<String>,
    } errors [NOT_FOUND];

    // ADR-0386
    // A library refusal carries `args` = {code, args, line?}: `code` is the fixed reason, `args` its arguments. Every
    //   refusal applied nothing (`library-config-write-failed` aside: the mount change is live but not saved, and
    //   repeating the same call saves again — ADR-0381 · ADR-0400 decision 14).

    /// The library's mounts now, in order — each with its `name`, its folder `path`, `default` (at the default place),
    /// `builtin` (the read-only mount `engram`, always last) and `state` (+ `state_why`; checked now, the built-in's
    /// read at start); `config`
    /// (library.json) = `ok` | `read_only` | `corrupt` | `unsaved` (+ `config_why`; `unsaved` = a mount change is live
    /// but not saved), `suggested_new` = the default place. `root_generation` = pass it to an edit, which then refuses
    /// if a mount changed since (CONFLICT `root-generation-stale`). CONFLICT `library-unavailable` only from a daemon
    /// that runs no library.
    #[effect(Read)]
    #[since(9)]
    "library.status" => args LibraryStatusArgs {} -> ok LibraryStatusOk {
        daemon_id: String,
        mounts: Vec<MountStatusRow>,
        config: String,
        config_why: Option<String>,
        suggested_new: String,
        root_generation: u64,
    } errors [CONFLICT];

    // ADR-0400
    // The mount commands answer the status after the change. A failed save (CONFLICT `library-config-write-failed`)
    //   leaves the change live: repeat the same call to save it — a repeat whose change is already live answers
    //   success. After a timeout, read `library.status` before repeating.

    /// Registers the absolute folder `path` as the mount `name` (`create` makes the folder once the overlap check passed — a
    /// refused check creates nothing; no file is copied or moved) and saves it in library.json; answers the status and `already_pointing` — references that already named
    /// `name` and now resolve into the folder. The same name already at the same folder only saves again.
    /// INVALID_ARGUMENT `mount-name-invalid` (one name: no `/ \ :`, no leading `.`, at most 64 characters, not
    /// `engram`) · `item-path-invalid` (a relative path); CONFLICT `mount-name-taken` · `mount-overlaps` (`with` =
    /// `mount` + `mount` — another mount's folder, inside or holding it; only mounts are judged) ·
    /// `library-config-read-only` (nothing done) · `library-root-unusable` (the folder cannot be used, or whether it
    /// overlaps could not be told — nothing published) · `library-unavailable` (the run's first overlap check of the
    /// saved mounts has not finished — nothing done; repeat the call) · `library-config-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "library.addMount" => args LibraryAddMountArgs {
        name: String,
        path: String,
        /// Create the folder first. Unspecified = false.
        create: Option<bool>,
    } -> ok LibraryAddMountOk {
        // ★The status fields must not differ by a character from `LibraryStatusOk`★ — the pairing destructures it
        //   (`library_commands.rs`); field meanings: `library.status`.
        daemon_id: String,
        mounts: Vec<MountStatusRow>,
        config: String,
        config_why: Option<String>,
        suggested_new: String,
        root_generation: u64,
        already_pointing: PointingRow,
    } errors [CONFLICT];

    /// Unregisters the mount `mount` — no file is touched, ever; adding it back restores every reference. Answers the
    /// status, `now_absent` — references that name it and now read as absent (a launch skips them with one notice) —
    /// and `outcome` = `removed`, or `saved_only` when the mount was already gone because an earlier removal's save
    /// failed (only library.json was written). NOT_FOUND `mount-not-found`; CONFLICT `mount-read-only` (`engram`) ·
    /// `library-config-read-only` · `library-unavailable` (as `library.addMount`) · `library-config-write-failed`;
    /// INVALID_ARGUMENT `mount-name-invalid`.
    #[effect(Write)]
    #[since(9)]
    "library.removeMount" => args LibraryRemoveMountArgs {
        mount: String,
    } -> ok LibraryRemoveMountOk {
        // ★The status fields must not differ by a character from `LibraryStatusOk`★ — the pairing destructures it
        //   (`library_commands.rs`); field meanings: `library.status`.
        daemon_id: String,
        mounts: Vec<MountStatusRow>,
        config: String,
        config_why: Option<String>,
        suggested_new: String,
        root_generation: u64,
        now_absent: PointingRow,
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Points the mount `mount` at the absolute folder `path` under the same name (`create` makes it first; no file is
    /// copied or moved; references do not change) and saves it in library.json; answers the status. Pointing it at the
    /// folder it already has only saves again. Refusals as `library.addMount` except `mount-name-taken`, judged against
    /// every other mount, plus NOT_FOUND `mount-not-found` and CONFLICT `mount-read-only` (`engram`).
    #[effect(Write)]
    #[since(9)]
    "library.relocateMount" => args LibraryRelocateMountArgs {
        mount: String,
        path: String,
        /// Create the folder first. Unspecified = false.
        create: Option<bool>,
    } -> ok LibraryRelocateMountOk {
        // ★The status fields must not differ by a character from `LibraryStatusOk`★ — the pairing destructures it
        //   (`library_commands.rs`); field meanings: `library.status`.
        daemon_id: String,
        mounts: Vec<MountStatusRow>,
        config: String,
        config_why: Option<String>,
        suggested_new: String,
        root_generation: u64,
    } errors [NOT_FOUND, CONFLICT];

    /// Renames the mount `mount` to `to` and re-points every reference to `mount/…` (in the preset files of every
    /// mount that can be read, then in agents) at `to/…`; a case-only rename is allowed; saves library.json. When
    /// files would be rewritten, or references would stay as they are because their mount cannot be read, the first
    /// call applies nothing and answers CONFLICT `mount-rename-unconfirmed` with `plan` (`files` per mount with `git`
    /// — the mount is in a git working tree —, `unread` mounts with `paths` — their files that refer to `mount/…` and
    /// stay as they are; null: never walked, what in it refers is unknown —, `agents`, `whole_files` — rewritten
    /// whole, comments and quoting not kept —,
    /// `already_pointing`), `root_generation` and `plan_digest` (over the files in `plan` and its never-walked mounts); repeat with `yes` and that
    /// `root_generation` and `plan_digest` — both required whenever the plan needs confirmation: files to rewrite or
    /// references left unread (either missing: the same refusal; stale: `root-generation-stale` with a fresh `plan` and
    /// `plan_digest`). To have the unread files rewritten too, wait for their mount and call again
    /// instead of confirming. A confirmed call whose plan is no longer the one confirmed — a preset came to refer to
    /// `mount/…` or stopped, a mount came back or became unreadable — applies nothing and answers
    /// `mount-rename-unconfirmed` with the fresh `plan`; read it and confirm again. Answers the status, the pairs `fixed`, the files
    /// `written` and `skipped` with why, `stopped` when an IO failure or the agents' save stopped it part-way (the old
    /// name stays and the answer carries no `plan_digest` — call again: the repeat plans what is left, and a plan that
    /// needs confirmation is answered as above to confirm with its own tokens, while the tokens confirmed before the stop
    /// answer `mount-rename-unconfirmed` or `root-generation-stale` with that plan unless it is the confirmed one
    /// again — every written file came to refer to `mount/…` again), and `already_pointing` — references
    /// that already named `to` (0 for a case-only rename where names ignore case). A `skipped` row whose path is a mount
    /// name: that mount was never walked, so what in it refers to `mount/…` is unknown. `git` is null when it could not
    /// be told. NOT_FOUND `mount-not-found`; CONFLICT `mount-unusable` (`{mount, state, why}` — the
    /// mount cannot be read, so its own references could not be rewritten) · `mount-name-taken` · `mount-read-only`
    /// (`engram`) · `library-config-read-only` · `library-unavailable` (as `library.addMount`; also when no first scan
    /// of the library could be applied, or a mount's folder — `mount` itself included — was lost during the rename:
    /// the old name stays, repeat after `library.status`) ·
    /// `library-config-write-failed`, or the agent store's refusal when it is read-only and agents would change;
    /// INVALID_ARGUMENT `mount-name-invalid`.
    #[effect(Write)]
    #[since(9)]
    "library.renameMount" => args LibraryRenameMountArgs {
        mount: String,
        to: String,
        /// Confirms the plan. Unspecified = false.
        yes: Option<bool>,
        /// The value the plan answered — required with `yes` whenever the plan needs confirmation (files to rewrite or
        /// references left unread).
        root_generation: Option<u64>,
        /// The `plan_digest` the plan answered — required with `yes` whenever the plan needs confirmation (files to
        /// rewrite or references left unread).
        plan_digest: Option<String>,
    } -> ok LibraryRenameMountOk {
        // ★The status fields must not differ by a character from `LibraryStatusOk`★ — the pairing destructures it
        //   (`library_commands.rs`); field meanings: `library.status`.
        daemon_id: String,
        mounts: Vec<MountStatusRow>,
        config: String,
        config_why: Option<String>,
        suggested_new: String,
        root_generation: u64,
        fixed: Vec<RelinkPairRow>,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        stopped: Option<RelinkStopRow>,
        already_pointing: PointingRow,
    } errors [NOT_FOUND, CONFLICT];

    /// Every folder of the library, flat, by path; a skill folder is marked `skill`; each mount (a top folder) carries
    /// its `state`. The folders of a mount that cannot be read are left out and the mount is named in `unavailable`
    /// (the built-in mount `engram` too when its folder cannot be read, though its rows stay as stand-ins and a launch
    /// goes on with `builtin-unavailable`). CONFLICT `library-unavailable` when no scan of the library could be applied.
    #[effect(Read)]
    #[since(9)]
    "library.folders" => args LibraryFoldersArgs {} -> ok LibraryFoldersOk {
        folders: Vec<LibraryFolderRow>,
        unavailable: Vec<UnavailableMountRow>,
    } errors [CONFLICT];

    /// Re-reads `folder` (unspecified = the whole library) from disk and reports it: `changed` items, `problems` of the
    /// files re-read, `no_kind` (yaml files with no `kind` — not listed), `skipped_links` (links not entered) and
    /// `errors` (where reading failed — those items are kept as they were; a mount that could not be read, or one
    /// registered or moved during the scan and so not walked by it, is a row with its name as `path`; a row with no
    /// `path` = the folder asked for could not be read — or is in the built-in mount `engram`, which is never scanned).
    /// Use it after changing
    /// library files outside Engram. It never refuses except for a `folder` that is
    /// not a library path (INVALID_ARGUMENT `item-path-invalid`).
    #[effect(Read)]
    #[since(9)]
    "library.refresh" => args LibraryRefreshArgs {
        folder: Option<String>,
    } -> ok LibraryRefreshOk {
        changed: u32,
        problems: Vec<PathReasonRow>,
        no_kind: Vec<String>,
        skipped_links: Vec<String>,
        errors: Vec<RefreshErrorRow>,
    } errors [];

    /// The items directly in `folder` (unspecified = the library's top, which holds only the mounts) and its plain
    /// subfolders. `query` searches name and
    /// description (case-insensitive) in every item below `folder` and answers matching rows only (no
    /// `folders`). `kinds` keeps only those kinds (unspecified or empty = all; a broken file counts as the kind it
    /// claims; a skill's `SKILL.md` matches only `SkillFile`). A `folder` at or below a link answers one
    /// `path-through-link` row, which the filters do not apply to. Unspecified `folder` lists the mounts, each with
    /// its `state` — the built-in read-only mount `engram` among them (`engram/default`, `engram/priming`, folder
    /// `engram/settings`); a `query` without `folder` searches the mounts that can be read and names the others in
    /// `unavailable` (the built-in mount `engram` too when its folder cannot be read, though its rows stay as
    /// stand-ins and a launch goes on with `builtin-unavailable`). CONFLICT `library-unavailable` (with `mount`) when
    /// `folder` is in a
    /// mount that cannot be read.
    #[effect(Read)]
    #[since(9)]
    "library.list" => args LibraryListArgs {
        folder: Option<String>,
        query: Option<String>,
        kinds: Option<Vec<LibraryKind>>,
    } -> ok LibraryListOk {
        folder: Option<String>,
        rows: Vec<ListRow>,
        folders: Vec<ListFolderRow>,
        unavailable: Vec<UnavailableMountRow>,
    } errors [CONFLICT];

    /// One item's detail: its row, its file on disk (`absolute`), the presets that list it (`presets_direct`) or reach
    /// it through nested presets (`presets_indirect`), the agents that attach it (`agents_direct` — a preset only) or a
    /// preset reaching it (`agents_indirect`), an instruction's `excerpt` and a settings file's field names
    /// (`settings_fields`). A built-in (`engram/…`) counts only the Default slot: `agents_direct` = the agents whose
    /// Default slot is on for `engram/default`, and the preset and indirect lists are empty (every other built-in: all
    /// empty). NOT_FOUND `not-found` = no such item (a mount's name is not an item); CONFLICT `library-unavailable` (with
    /// `mount` when the item's mount cannot be read).
    #[effect(Read)]
    #[since(9)]
    "library.read" => args LibraryReadArgs {
        path: String,
    } -> ok LibraryReadOk {
        row: ListRow,
        absolute: String,
        presets_direct: Vec<String>,
        presets_indirect: Vec<String>,
        agents_direct: Vec<AgentRefRow>,
        agents_indirect: Vec<AgentRefRow>,
        excerpt: Option<String>,
        settings_fields: Vec<String>,
    } errors [NOT_FOUND, CONFLICT];

    /// Creates folder `name` in `parent` (unspecified: INVALID_ARGUMENT `virtual-root-read-only` — nothing is created
    /// at the library's top; a mount's name alone is a parent like any folder); answers its path. CONFLICT
    /// `name-exists` (nothing replaced) · `library-unavailable` (with `mount` when the parent's mount cannot be read — nothing is written into
    /// it) · `mount-read-only` (`{mount, path}` — the parent is in the built-in mount `engram`) ·
    /// `root-generation-stale` · `library-write-failed` (a retry after it may answer `name-exists` — the partial file
    /// or folder stays); NOT_FOUND `not-found` = the parent is missing.
    #[effect(Write)]
    #[since(9)]
    "library.newFolder" => args LibraryNewFolderArgs {
        parent: Option<String>,
        name: String,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryNewFolderOk {
        path: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Creates file `name` of `kind` in `folder` (unspecified: refused as `library.newFolder`) from an empty template (`.md` or `.yaml` is
    /// added unless the name ends with it); answers its path. Refusals as `library.newFolder`.
    #[effect(Write)]
    #[since(9)]
    "library.newFile" => args LibraryNewFileArgs {
        folder: Option<String>,
        name: String,
        kind: LibraryNewFileKind,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryNewFileOk {
        path: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Re-points every reference to `from` (in preset files, then agents) at `to`; when `from` is missing it also
    /// re-points the other missing items of `from`'s folder whose names are in `to`'s folder (`more` of them). Answers
    /// the pairs `fixed`, the files `written`, the files `skipped` with why, and `stopped` when an IO failure stopped
    /// it after a write (run the same relink again once it is cleared). NOT_FOUND `not-found` = `to` is not in the
    /// library; INVALID_ARGUMENT `relink-kind-mismatch` = `to` is of another kind than its references need; CONFLICT
    /// `library-unavailable` (with `mount` when `to`'s mount cannot be read) · `mount-read-only` (`{mount, path}` —
    /// `from` or `to` is in the built-in mount `engram`) · `root-generation-stale`. A file in a
    /// mount that cannot be read is not rewritten: it is in `skipped` with `library-unavailable` and its `mount`. A
    /// `skipped` row whose `path` is a mount's name means that mount could not be read at all: references inside it
    /// may remain (`fixed` covers the mounts that were read) — run the same relink again once it is back.
    #[effect(Write)]
    #[since(9)]
    "library.relink" => args LibraryRelinkArgs {
        from: String,
        to: String,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryRelinkOk {
        fixed: Vec<RelinkPairRow>,
        more: u32,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        stopped: Option<RelinkStopRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0369
    // ADR-0409
    // ADR-0419: the five file writes take the launch deadline class — a copy across volumes can outlast the ordinary
    //   wait; `library.listTrash` is bounded by the read budget and stays ordinary.

    /// Moves `paths` into the folder `to_folder`, each keeping its name — on one volume a rename that never replaces,
    /// across volumes a copy whose original then goes into its own mount's trash. A taken name follows `on_conflict`
    /// (unspecified: that item's `conflict` `name-exists`); on one volume same-name folders merge entry by entry, across
    /// volumes they are refused (`crosses-volumes`). Every preset file that names a moved path is rewritten whole, then
    /// the agents are re-pointed — no confirmation, except when a skill's lone `SKILL.md` leaves its skill folder (or a
    /// merge would leave a referenced skill folder without it): then nothing is applied and the answer is CONFLICT
    /// `skill-unconfirmed` with `plan` (`presets`, `agents`), `plan_digest` and `forms` — repeat with `yes`
    /// (references kept) or `yes` + `detach_refs` + that `plan_digest` (references removed). Answers one row per path in
    /// the order given (`items`), the old → new `mapping` the references followed (apply in order), the preset files
    /// `written` and `skipped` with why, the git working `trees` they lie in, the `agents` re-pointed, `detached`
    /// skill folders, and `stopped` when the reference fix stopped part-way (the items stay moved; `library.relink` the
    /// old path once it is cleared). INVALID_ARGUMENT `item-path-invalid` (`to_folder` is no library path or holds a dot
    /// or temp name) · `detach-refs-invalid` (`detach_refs` with no such skill); CONFLICT `mount-read-only`
    /// (`to_folder` in the built-in mount `engram`) · `library-unavailable` · `skill-unconfirmed` ·
    /// `root-generation-stale` (either met after copies across volumes also names them in `args.args.stagings`), or the
    /// agent store's refusal when it is read-only and agents would change. Can take long (a copy across volumes); after
    /// an answer that never came the outcome is unknown — do not repeat; read the destination after a pause.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.move" => args LibraryMoveArgs {
        /// Library paths of the items to move (a file, or a folder with everything in it).
        paths: Vec<String>,
        /// The library path of the folder they go into (it must exist).
        to_folder: String,
        /// Unspecified = a taken name is that item's conflict.
        on_conflict: Option<LibraryOnConflict>,
        /// Confirms a `skill-unconfirmed` plan. Unspecified = false.
        yes: Option<bool>,
        /// With `yes` and `plan_digest`: removes the references the plan named. Unspecified = false.
        detach_refs: Option<bool>,
        /// The `plan_digest` the plan answered — required with `detach_refs`.
        plan_digest: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryMoveOk {
        items: Vec<MoveItemRow>,
        mapping: Vec<MovePairRow>,
        detached: Vec<String>,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        trees: Vec<GitTreeRow>,
        agents: Vec<AgentRefRow>,
        stopped: Option<RelinkStopRow>,
    } errors [CONFLICT];

    /// Renames the item `path` to `name` in its folder (a case-only rename too) — a move to a new name, answered as
    /// `library.move` answers (one row): a taken name is the row's `conflict` `name-exists` (nothing replaced), its own
    /// spelling is `skipped` `in-place`; references follow, and a skill's `SKILL.md` renamed so its folder is no skill
    /// asks first (`skill-unconfirmed`, as `library.move`). Refusals as `library.move`, with `name` — not one valid
    /// name, or a dot or temp name: INVALID_ARGUMENT `item-path-invalid` — in place of `to_folder`.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.rename" => args LibraryRenameArgs {
        /// The library path of the item.
        path: String,
        /// Its new name (one name, no folder).
        name: String,
        /// Confirms a `skill-unconfirmed` plan. Unspecified = false.
        yes: Option<bool>,
        /// With `yes` and `plan_digest`: removes the references the plan named. Unspecified = false.
        detach_refs: Option<bool>,
        /// The `plan_digest` the plan answered — required with `detach_refs`.
        plan_digest: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryRenameOk {
        // ★The fields must not differ by a character from `LibraryMoveOk`★ — the pairing destructures it
        //   (`library_commands.rs`); field meanings: `library.move`.
        items: Vec<MoveItemRow>,
        mapping: Vec<MovePairRow>,
        detached: Vec<String>,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        trees: Vec<GitTreeRow>,
        agents: Vec<AgentRefRow>,
        stopped: Option<RelinkStopRow>,
    } errors [CONFLICT];

    /// Copies `paths` into the folder `to_folder` — each keeps its name (a built-in item from `engram` takes its file's
    /// name); copied into its own folder it takes the copy name (`a - Copy.md`, then `a - Copy (2).md` …, whatever
    /// `on_conflict` says). A taken name follows `on_conflict` (unspecified: that item's `conflict` `name-exists`); a
    /// same-name folder is never merged (`copy-merge-refused`). Each copy is made aside under a temp name and then put
    /// in place; nothing refers to a copy, so no reference changes. Answers one row per path in the order given; a copy
    /// that failed part-way names its staging in `args` (`staging_row` — it went into the trash —, or `staging` ·
    /// `staging_disk` · `staging_kept` when it stayed). INVALID_ARGUMENT `item-path-invalid` (as `library.move`);
    /// CONFLICT `mount-read-only` · `library-unavailable` · `root-generation-stale` (met after copying:
    /// `args.args.stagings`). Can take long; after an answer that never came the outcome is unknown — do not repeat;
    /// read the destination after a pause.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.copy" => args LibraryCopyArgs {
        /// Library paths of the items to copy, or built-in items (`engram/…`).
        paths: Vec<String>,
        /// The library path of the folder the copies go into (it must exist).
        to_folder: String,
        /// Unspecified = a taken name is that item's conflict.
        on_conflict: Option<LibraryOnConflict>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryCopyOk {
        items: Vec<CopyItemRow>,
    } errors [CONFLICT];

    /// Deletes `paths` into each one's own mount's trash (`.trash`, one bundle per mount per call — `library.listTrash`
    /// shows it, `library.restore` brings rows back). When preset files or agents refer to what goes, the first call
    /// applies nothing and answers CONFLICT `delete-unconfirmed` with `plan` (`presets` — the preset files listing a
    /// deleted path —, `agents`), `plan_digest` and `forms`: repeat with `yes` (references kept — they read as not
    /// found until a restore) or `yes` + `detach_refs` + that `plan_digest` (their lines and attachments removed). A plan
    /// that changed before the repeat answers the same CONFLICT with the fresh plan, nothing applied. Answers one row per
    /// path in the order given (`items`), and with `detach_refs` the paths `detached`, the preset files `written` and
    /// `skipped` with why, the `agents` changed and `stopped` when that fix stopped part-way (the items stay deleted).
    /// CONFLICT `library-unavailable` · `delete-unconfirmed` · `root-generation-stale`, or the agent store's refusal
    /// when it is read-only and agents would change. Emptying the trash is not a command of this list.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.delete" => args LibraryDeleteArgs {
        /// Library paths of the items to delete (a file, or a folder with everything in it).
        paths: Vec<String>,
        /// Confirms a `delete-unconfirmed` plan. Unspecified = false.
        yes: Option<bool>,
        /// With `yes` and `plan_digest`: removes the references the plan named. Unspecified = false.
        detach_refs: Option<bool>,
        /// The `plan_digest` the plan answered — required with `detach_refs`.
        plan_digest: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryDeleteOk {
        items: Vec<DeleteItemRow>,
        detached: Vec<String>,
        written: Vec<String>,
        skipped: Vec<PathReasonRow>,
        agents: Vec<AgentRefRow>,
        stopped: Option<RelinkStopRow>,
    } errors [CONFLICT];

    /// The trash of every registered mount, in mount order: its bundles by id (one per delete per mount), each with
    /// when it was deleted and its rows — `original` is where `library.restore` puts a row back and `restorable` says
    /// whether it can. A mount whose trash cannot be read is named with its `problem`, not waited for. Reads only.
    /// CONFLICT `library-unavailable` only from a daemon that runs no library.
    #[effect(Read)]
    #[since(9)]
    "library.listTrash" => args LibraryListTrashArgs {} -> ok LibraryListTrashOk {
        mounts: Vec<MountTrashRow>,
    } errors [CONFLICT];

    /// Restores trash `rows` (as `library.listTrash` lists them) to the same paths in the same mounts, making missing
    /// parent folders; nothing is ever overwritten — a taken name is that row's `conflict` `name-exists`, and with
    /// `keep_both` the row comes back as `name (2)`, `name (3)` … instead. Answers one row per given row in the order
    /// given. CONFLICT `library-unavailable` · `root-generation-stale`.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.restore" => args LibraryRestoreArgs {
        /// The trash rows to restore.
        rows: Vec<TrashAddressRow>,
        /// A taken name restores under the next free keep-both name. Unspecified = false.
        keep_both: Option<bool>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok LibraryRestoreOk {
        items: Vec<RestoreItemRow>,
    } errors [CONFLICT];

    // ADR-0322 decision 1
    // ADR-0408
    // ADR-0419: shell only — named in `SHELL_ONLY`, so the daemon refuses it from every control route and leaves it out
    //   of the control route's list; the library screen sends it once the user has confirmed.

    /// Removes trash rows for good — nothing goes to an OS Recycle Bin: `bundles` empties each bundle of exactly the rows
    /// the trash view listed for it (what it did not list stays, with the record), `rows` deletes single rows for good.
    /// Answers every addressed row (the bundles' rows in the order given, then `rows`) and every addressed bundle with
    /// its outcome; one that is not gone carries why. INVALID_ARGUMENT `nothing-to-empty` (neither `bundles` nor
    /// `rows`); CONFLICT `library-unavailable` only from a daemon that runs no library; UNSUPPORTED `shell-only` from
    /// any caller but the Engram app. Can take long; after an answer that never came, read the trash view again rather
    /// than resend.
    #[effect(Write)]
    #[since(9)]
    #[deadline(Launch)]
    "library.emptyTrash" => args LibraryEmptyTrashArgs {
        /// Empty-all: bundles, each with the rows the view listed for it.
        bundles: Option<Vec<ListedBundleArg>>,
        /// Permanent delete: single trash rows.
        rows: Option<Vec<TrashAddressRow>>,
    } -> ok LibraryEmptyTrashOk {
        rows: Vec<EmptiedRow>,
        bundles: Vec<EmptiedBundleRow>,
    } errors [CONFLICT, UNSUPPORTED];

    // ADR-0358
    // ADR-0384
    // ADR-0386
    // A preset edit rewrites the whole file (hand-written comments and layout are not kept) and answers the preset as
    //   committed. A path that is not a library path is INVALID_ARGUMENT `item-path-invalid`; a file that is not a
    //   preset, INVALID_ARGUMENT `wrong-kind` or the reader's code; a preset in a mount that cannot be read, CONFLICT
    //   `library-unavailable` with `mount` (ADR-0400 decision 6); a check or edit of a path in the built-in mount
    //   `engram`, CONFLICT `mount-read-only` with `mount` and `path` (ADR-0400 decision 20).

    /// A preset's entries with their state, its folder entries, the conflicts and problems a launch of it alone would
    /// meet, its edition `rev` and how many agents attach it. `path` = a library path or `engram/default`. NOT_FOUND
    /// `not-found`; INVALID_ARGUMENT `wrong-kind` (not a preset); CONFLICT `library-unavailable` (with `mount` when the
    /// preset's mount cannot be read).
    #[effect(Read)]
    #[since(9)]
    "preset.get" => args PresetGetArgs {
        path: String,
    } -> ok PresetGetOk {
        // ★The fields must not differ by a character from `PresetView`★ — the pairing is `From<PresetView>`
        //   (destructuring); field meanings: `PresetView`.
        path: String,
        rev: String,
        description: Option<String>,
        presets: Vec<PresetEntryRow>,
        instructions: Vec<PresetEntryRow>,
        skills: Vec<PresetEntryRow>,
        settings: Vec<PresetEntryRow>,
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
        agents: u32,
    } errors [NOT_FOUND, CONFLICT];

    /// Judges a draft of the preset's lists as `preset.save` would, without writing: the folders it would list, the
    /// conflicts and problems a launch of it would meet, and `save_refuses` = a save of this draft would be refused
    /// (each mismatch is a `broken` problem with `args` `{path, list}`). `folders` omitted = the file's folders stay.
    /// CONFLICT `library-unavailable` (with `mount` when the preset's mount cannot be read) · `mount-read-only` (a
    /// path in the built-in mount `engram`).
    #[effect(Read)]
    #[since(9)]
    "preset.check" => args PresetCheckArgs {
        path: String,
        presets: Vec<PresetEntryArg>,
        instructions: Vec<PresetEntryArg>,
        skills: Vec<PresetEntryArg>,
        settings: Vec<PresetEntryArg>,
        folders: Option<Vec<String>>,
    } -> ok PresetCheckOk {
        save_refuses: bool,
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
    } errors [CONFLICT];

    /// Adds `items` (each into the list of its kind, `backend` for all of them) or absolute `folders` — one of the two —
    /// after the `after` entry (else at the end). Applies what it can: each item it cannot add is in `skipped` with why
    /// (`not-found`, `wrong-kind`, `already-present`, `preset-cycle`, a backend word it does not take …). A bad folder
    /// refuses the whole call, INVALID_ARGUMENT `preset-folders-refused` naming each. NOT_FOUND `not-found` = no such
    /// preset file; CONFLICT `library-unavailable` · `mount-read-only` · `root-generation-stale` ·
    /// `library-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "preset.add" => args PresetAddArgs {
        path: String,
        items: Option<Vec<String>>,
        /// With `items` only. Unspecified = every backend.
        backend: Option<String>,
        folders: Option<Vec<String>>,
        after: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok PresetAddOk {
        /// What was added (folders normalized).
        applied: Vec<String>,
        skipped: Vec<SkippedRow>,
        view: PresetView,
    } errors [NOT_FOUND, CONFLICT];

    /// Removes `items` (every entry of each, in every list) or `folders` — one of the two; removing what is not there
    /// changes nothing. Answers the preset after the change. NOT_FOUND `not-found` = no such preset file; CONFLICT
    /// `library-unavailable` · `mount-read-only` · `root-generation-stale` · `library-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "preset.remove" => args PresetRemoveArgs {
        path: String,
        items: Option<Vec<String>>,
        folders: Option<Vec<String>>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok PresetRemoveOk {
        // ★The fields must not differ by a character from `PresetView`★ (`From<PresetView>`).
        path: String,
        rev: String,
        description: Option<String>,
        presets: Vec<PresetEntryRow>,
        instructions: Vec<PresetEntryRow>,
        skills: Vec<PresetEntryRow>,
        settings: Vec<PresetEntryRow>,
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
        agents: u32,
    } errors [NOT_FOUND, CONFLICT];

    /// Puts one `section` (`presets` | `instructions` | `skills` | `settings` | `folders` — another word is
    /// INVALID_ARGUMENT) in `order` — every entry `preset.get` shows in it, each once (a duplicate as often as it is
    /// listed), else INVALID_ARGUMENT `reorder-order-mismatch` with the expected order in `args`. `base_rev` = the `rev` you
    /// read; CONFLICT `rev-mismatch` = the file changed since (read it again). NOT_FOUND `not-found`; CONFLICT
    /// `library-unavailable` · `mount-read-only` · `root-generation-stale` · `library-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "preset.reorder" => args PresetReorderArgs {
        path: String,
        /// `presets` | `instructions` | `skills` | `settings` | `folders` — the preset file's own keys; another word is
        /// INVALID_ARGUMENT.
        section: String,
        order: Vec<String>,
        base_rev: String,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok PresetReorderOk {
        // ★The fields must not differ by a character from `PresetView`★ (`From<PresetView>`).
        path: String,
        rev: String,
        description: Option<String>,
        presets: Vec<PresetEntryRow>,
        instructions: Vec<PresetEntryRow>,
        skills: Vec<PresetEntryRow>,
        settings: Vec<PresetEntryRow>,
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
        agents: u32,
    } errors [NOT_FOUND, CONFLICT];

    /// Sets the backend of `items` in instructions and skills (`backend` unspecified = every backend). Applies what it
    /// can: an item the preset does not list, or lists only in presets or settings, is in `skipped`. An unknown backend
    /// word refuses the whole call, INVALID_ARGUMENT `backend-tag-unknown`. NOT_FOUND `not-found`; CONFLICT
    /// `library-unavailable` · `mount-read-only` · `root-generation-stale` · `library-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "preset.setBackend" => args PresetSetBackendArgs {
        path: String,
        items: Vec<String>,
        backend: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok PresetSetBackendOk {
        applied: Vec<String>,
        skipped: Vec<SkippedRow>,
        view: PresetView,
    } errors [NOT_FOUND, CONFLICT];

    /// Replaces the four lists (and `folders`, when given — omitted = they stay) with the draft, all or nothing: any
    /// mismatch (not a path, an unknown backend word, one item twice in a list, an item of another kind, a cycle, a
    /// relative folder …) refuses the whole save, INVALID_ARGUMENT `preset-save-refused` listing each. A missing item
    /// is allowed. `base_rev` (optional) = the `rev` you read; CONFLICT `rev-mismatch` = the file changed since.
    /// NOT_FOUND `not-found`; CONFLICT `library-unavailable` · `mount-read-only` · `root-generation-stale` ·
    /// `library-write-failed`.
    #[effect(Write)]
    #[since(9)]
    "preset.save" => args PresetSaveArgs {
        path: String,
        presets: Vec<PresetEntryArg>,
        instructions: Vec<PresetEntryArg>,
        skills: Vec<PresetEntryArg>,
        settings: Vec<PresetEntryArg>,
        folders: Option<Vec<String>>,
        base_rev: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok PresetSaveOk {
        // ★The fields must not differ by a character from `PresetView`★ (`From<PresetView>`).
        path: String,
        rev: String,
        description: Option<String>,
        presets: Vec<PresetEntryRow>,
        instructions: Vec<PresetEntryRow>,
        skills: Vec<PresetEntryRow>,
        settings: Vec<PresetEntryRow>,
        folders: Vec<FolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
        agents: u32,
    } errors [NOT_FOUND, CONFLICT];

    /// What a folder entry would load: the absolute `folder` normalized (`path`, `null` = not absolute), why
    /// `preset.add` would refuse it (`refused`), and per backend (`backend`, unspecified = every one) and mode (`mode`,
    /// unspecified = both) what that backend loads from it. `truncated` = the listing stopped at its cap, `skipped` =
    /// entries it could not read, `unlisted` = why a folder that exists could not be listed. Reads only.
    /// INVALID_ARGUMENT `backend-tag-unknown`; CONFLICT `library-unavailable` only from a daemon that runs no library.
    // ADR-0341
    #[effect(Read)]
    #[since(9)]
    "preset.previewFolder" => args PresetPreviewFolderArgs {
        folder: String,
        backend: Option<String>,
        mode: Option<AgentOutputFormat>,
    } -> ok PresetPreviewFolderOk {
        path: Option<String>,
        refused: Option<PreviewRefusalRow>,
        loads: Vec<PreviewLoadRow>,
        truncated: bool,
        skipped: Vec<String>,
        unlisted: Option<String>,
    } errors [CONFLICT];

    // ADR-0384
    // ADR-0386
    // An agent-preset edit answers the agent's presets as committed. `target` = the agent's name or id (NOT_FOUND /
    //   CONFLICT with candidates when it points at none or several); NOT_FOUND `not-found` `{agent}` = the agent was
    //   deleted meanwhile; INTERNAL `profile-edit-unavailable` = this daemon cannot write agents' presets.

    /// An agent's presets: the Default slot, the attached presets, its working folder and why it is locked, its backend
    /// and mode, `base_rev`, and the folders, conflicts, problems and notices a launch would meet — it leaves out the
    /// `missing` problem rows (and the `library_unavailable` row of a library mount that is not registered or not
    /// there) and refuses on the others.
    #[effect(Read)]
    #[since(9)]
    "agent.presets" => args AgentPresetsArgs {
        target: String,
    } -> ok AgentPresetsOk {
        // ★The fields must not differ by a character from `AgentPresetsView`★ — the pairing is
        //   `From<AgentPresetsView>` (destructuring); field meanings: `AgentPresetsView`.
        agent_id: String,
        name: String,
        default_preset: bool,
        presets: Vec<String>,
        cwd: String,
        workspace_lock: Option<String>,
        backend: String,
        mode: AgentOutputFormat,
        base_rev: String,
        takes_presets: bool,
        folders: Vec<AgentFolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Attaches `presets` (library presets; `engram/default` turns the Default slot on) after the `after` preset in
    /// call order (else at the end). Applies what it can: each one it cannot attach is in `skipped` with why
    /// (`not-found`, `wrong-kind`, `already-present`, `item-path-invalid`). CONFLICT `library-unavailable` (a library
    /// preset named while the library cannot be read — with `mount` when that preset's mount cannot be read) ·
    /// `root-generation-stale` applies nothing.
    #[effect(Write)]
    #[since(9)]
    "agent.attachPreset" => args AgentAttachPresetArgs {
        target: String,
        presets: Vec<String>,
        after: Option<String>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok AgentAttachPresetOk {
        applied: Vec<String>,
        skipped: Vec<SkippedRow>,
        view: AgentPresetsView,
    } errors [NOT_FOUND, CONFLICT];

    /// Detaches `presets` (every attached entry of each; `engram/default` turns the Default slot off — CONFLICT
    /// `default-preset-detach` unless `yes`). Detaching what is not attached changes nothing; an entry that is not a
    /// library path is matched by its exact text, and one that matches nothing refuses, INVALID_ARGUMENT
    /// `item-path-invalid`. CONFLICT `root-generation-stale`.
    #[effect(Write)]
    #[since(9)]
    "agent.detachPreset" => args AgentDetachPresetArgs {
        target: String,
        presets: Vec<String>,
        /// Confirms turning the Default slot off. Unspecified = false.
        yes: Option<bool>,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok AgentDetachPresetOk {
        // ★The fields must not differ by a character from `AgentPresetsView`★ (`From<AgentPresetsView>`).
        agent_id: String,
        name: String,
        default_preset: bool,
        presets: Vec<String>,
        cwd: String,
        workspace_lock: Option<String>,
        backend: String,
        mode: AgentOutputFormat,
        base_rev: String,
        takes_presets: bool,
        folders: Vec<AgentFolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Puts the attached presets in `order` — every entry `agent.presets` shows, each once (a duplicate as often as it
    /// is attached), else INVALID_ARGUMENT `reorder-order-mismatch` with the expected order in `args`. `base_rev` = the
    /// `base_rev` you read; CONFLICT `rev-mismatch` = the attachments changed since (read them again). The Default slot
    /// is untouched. CONFLICT `root-generation-stale`.
    #[effect(Write)]
    #[since(9)]
    "agent.reorderPresets" => args AgentReorderPresetsArgs {
        target: String,
        order: Vec<String>,
        base_rev: String,
        /// The value `library.status` answered — refuses if a mount changed since. Unspecified = the current mounts.
        root_generation: Option<u64>,
    } -> ok AgentReorderPresetsOk {
        // ★The fields must not differ by a character from `AgentPresetsView`★ (`From<AgentPresetsView>`).
        agent_id: String,
        name: String,
        default_preset: bool,
        presets: Vec<String>,
        cwd: String,
        workspace_lock: Option<String>,
        backend: String,
        mode: AgentOutputFormat,
        base_rev: String,
        takes_presets: bool,
        folders: Vec<AgentFolderRow>,
        conflicts: Vec<ProblemRow>,
        problems: Vec<ProblemRow>,
        notices: Vec<LaunchNoticeRow>,
    } errors [NOT_FOUND, CONFLICT];
}

/// Commands that **move the input** of a live agent — the common entry looks at the input lease (`check_input`) first.
///
/// ★Kept here rather than as a declaration-macro field★ — a marker on `CommandSpec` would make the command crate know the domain of input leases. Every name must be a `Write` declaration of this block (a test measures it).
/// The query (`agent.listQueuedInputs`) is a read and skips the lease; interrupt (`agent.interrupt`) is included, like WS `Interrupt`.
// ADR-0231
// ADR-0237
pub const INPUT_AFFECTING: &[&str] = &["agent.cancelQueuedInput", "agent.interrupt"];

/// Commands only the shell runs — the daemon's one entrance refuses them from a caller with no connection (every control
/// route) and the control route's command list omits them; WS `ListCommands`, the bindings and the schema keep them (the
/// shell finds them there).
///
/// ★Kept here rather than as a declaration-macro field★, as [`INPUT_AFFECTING`]. Every name must be a `Write` declaration
/// of this block outside the `agent` family (`/control/agent` builds only `agent.<verb>` names; a test measures both).
// ADR-0322 decision 1
// ADR-0419
pub const SHELL_ONLY: &[&str] = &["library.emptyTrash"];

/// One roster row — only what this table sees from the manager. `live` = `Some(status)` means a session is attached (liveness = [`AgentStatus::is_live`] alone, ADR-0116).
pub struct AgentRosterRow {
    pub id: AgentId,
    pub canonical_name: String,
    pub cwd: String,
    pub parent: Option<AgentId>,
    pub live: Option<AgentStatus>,
}

/// The result of a start — the fields this table reads.
pub struct StartedAgent {
    pub id: AgentId,
    pub name: String,
    pub status: AgentStatus,
    /// The launch notices for the reply — [`crate::manager::Activated::notices`].
    pub notices: Vec<Notice>,
}

impl From<Activated> for StartedAgent {
    fn from(Activated { info, notices }: Activated) -> Self {
        Self {
            id: info.id,
            name: info.name,
            status: info.status,
            notices,
        }
    }
}

/// **Everything** the `agent.*` handlers have the manager do.
///
/// ★Why a trait★: taking `AgentManager` as is would drag in a real PTY process to assert one handler — a violation of ADR-0012 「cut external dependencies with a seam」 (form rule T-1). With this seam the whole table is verified with a fake manager.
/// ★Implementations only translate★: the verb rules (target resolution · mode derivation · outcome translation) are held **by the handlers**; rules in implementations would make real and fake run different rules and the harness guard nothing.
// ADR-0155
// ADR-0012
pub trait AgentCommandHost: Send + Sync {
    fn roster(&self) -> Vec<AgentRosterRow>;
    /// The current state of the roster store (the profile file) — `agent.list` spreads it into [`RosterStoreRow`].
    // ADR-0291 R17 (D6)
    fn store_status(&self) -> StoreStatus;
    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile>;
    /// ★The roster-changing methods (`create_agent` · `rename_agent` · `reparent_agent` · `create_and_activate` ·
    ///   `duplicate_agent` · `set_workspace`) are changes with a caller★ — if the store refuses the change the roster
    ///   stays as it was and the answer is [`PtyError::Store`] (`set_workspace`: [`WorkspaceRefusal::Store`]) (the
    ///   manager's `try_` variants · ADR-0291 R17). `create_and_activate` is partial: a refused add gives `created:
    ///   false`; a failure after the add gives `created: true` and the row stays.
    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError>;
    /// [`AgentCommandHost::create_agent`] whose registry insert first asks `still`, inside the registry's lock —
    /// `Ok(None)` = it answered `false` and nothing was registered; every lock is released on return
    /// ([`AgentManager::try_create_agent_if`]). `agent.new {presets}` passes the library change-number comparison
    /// (ADR-0400 decision 18). No default: an implementation that asks `still` outside the lock its insert takes
    /// would let a library change land between the answer and the insert.
    fn create_agent_if(
        &self,
        profile: AgentProfile,
        still: &dyn Fn() -> bool,
    ) -> Result<Option<AgentProfile>, PtyError>;
    /// The handler puts the returned notices on its success reply, so the activation counts as delivering them
    /// ([`crate::manager::NoticeDelivery::Delivered`] — a start marks the set as shown for this agent).
    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError>;
    /// Create-and-start for a profile whose id is not in the roster ([`AgentManager::create_and_activate`]) — the
    /// host call of [`create_and_start`]. `delivery` = whether the caller shows the returned notices.
    /// `Err(NotStarted { created: true, .. })` = the agent was registered and stays asleep.
    // ADR-0348
    fn create_and_activate(
        &self,
        profile: &AgentProfile,
        delivery: NoticeDelivery,
    ) -> Result<Activated, NotStarted>;
    fn rename_agent(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<RenameOutcome, PtyError>;
    /// ★`Ok(false)` does not tell the reason★ — the manager gives only one bool (`ProfileRegistry::try_reparent`).
    /// Absence and a structural conflict are told apart by the handler **by re-querying the roster** (`verb_move`).
    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> Result<bool, PtyError>;
    /// `agent.setWorkspace` ([`AgentManager::try_set_workspace`]) — `cwd` as typed; the manager normalizes and stats it.
    // ADR-0386
    fn set_workspace(&self, id: AgentId, cwd: &str) -> Result<WorkspaceSet, WorkspaceRefusal>;
    /// `agent.duplicate` ([`AgentManager::try_duplicate_agent`]) — the registered copy. The handler asks the backend
    /// policy before calling.
    // ADR-0386
    fn duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError>;
    /// The waiting list of the live incarnation. `None` = no live session.
    // ADR-0231
    fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing>;
    /// Cancels one queued input of the live incarnation. `CancelError::NotFound` if there is no live session.
    // ADR-0231
    fn cancel_queued_input(
        &self,
        id: AgentId,
        input_id: &str,
    ) -> Result<CancelOutcome, CancelError>;
    /// Interrupts the running turn of the live incarnation. `PtyError::NotFound` if no live session · `PtyError::Unsupported` if no turn to interrupt or the link cannot interrupt.
    // ADR-0237
    fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError>;
}

/// The exit (port) telling attached clients the roster changed.
///
/// ★Missing, the dashboard tree **shows the old roster until an unrelated event comes**★ (silent stale). A verb that changed the roster's **composition** must call it — that differs per verb (waking does not change composition), so the handler carries it.
/// ★A port, not a dependency of the table★: the real notification is an all-connection fan-out owned by the daemon; the consumer holds a narrow trait and the assembly provides the adapter (like daemon `control::RosterBroadcast`).
/// Must be non-blocking — the fan-out only does try_send to per-connection queues.
// ADR-0155
pub trait RosterChanged: Send + Sync {
    fn roster_changed(&self);
}

/// **Everything** the `usage.*` handlers have the daemon's usage service do (a port) — the real thing = daemon `UsageService`.
///
/// ★Synchronous — called on a blocking pool thread (TRD S21 usage-limit-slot §3 #17)★: the daemon's common entry drives only handlers that finish at the first poll (`call_daemon_command`); an async handler that waits becomes `OUTCOME_UNKNOWN`.
///   So both methods wait for the query to end **on the calling thread** (cap 5 s) on a std channel — no async runtime here.
/// ★The field is found by [`usage_vendor_of`], not the incoming spelling★ — the bus word (`Claude`) and the wire word (`claude`) must hit one field (§3 #43).
/// `None` = this daemon holds no field of that backend — the handler answers `NOT_FOUND`.
// ADR-0155
// ADR-0012
pub trait UsageCommandHost: Send + Sync {
    /// The `Get` of the request table — before the next automatic query time, it answers with the current value without querying.
    fn get(&self, backend: &AgentBackend) -> Option<UsageVendorRow>;
    /// The `Refresh` of the request table — within the refusal period, it answers with the current value without querying.
    fn refresh(&self, backend: &AgentBackend) -> Option<UsageVendorRow>;
}

/// Bus word → the vendor key of the usage field; `None` = no prober for that word.
///
/// ★Same path as the wire entry★ — both find the prober with [`usage_probe_for`](crate::backend::usage_probe_for) and take its key as the field (wire-side pair: daemon `connection_core`); a key from the incoming spelling would hit different fields.
// ADR-0004
pub fn usage_vendor_of(backend: &AgentBackend) -> Option<UsageVendorKey> {
    crate::backend::usage_probe_for(&backend_word(backend)).map(|probe| probe.key())
}

/// The return of `usage.refresh` — its fields are the same as `UsageVendorRow` (the declaration's comment). ★Moved by destructuring (without `..`)★ — if either
/// side gains a field, compilation stops here.
impl From<UsageVendorRow> for UsageRefreshOk {
    fn from(row: UsageVendorRow) -> Self {
        let UsageVendorRow {
            backend,
            account_key,
            plan,
            windows,
            in_flight,
            served,
            state,
            next_attempt_in_secs,
            retry_in_secs,
            detail_kind,
            detail_code,
            upstream,
        } = row;
        Self {
            backend,
            account_key,
            plan,
            windows,
            in_flight,
            served,
            state,
            next_attempt_in_secs,
            retry_in_secs,
            detail_kind,
            detail_code,
            upstream,
        }
    }
}

impl AgentCommandHost for AgentManager {
    fn roster(&self) -> Vec<AgentRosterRow> {
        AgentManager::roster(self)
            .into_iter()
            .map(|entry| AgentRosterRow {
                id: entry.id,
                canonical_name: entry.canonical_name,
                cwd: entry.cwd,
                parent: entry.parent,
                live: entry.live.map(|info| info.status),
            })
            .collect()
    }

    fn store_status(&self) -> StoreStatus {
        self.agents_store_status()
    }

    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        AgentManager::agent_snapshot(self, id)
    }

    // ADR-0291 R17
    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        AgentManager::try_create_agent(self, profile)
    }

    // ADR-0400 decision 18
    fn create_agent_if(
        &self,
        profile: AgentProfile,
        still: &dyn Fn() -> bool,
    ) -> Result<Option<AgentProfile>, PtyError> {
        AgentManager::try_create_agent_if(self, profile, still)
    }

    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError> {
        AgentManager::activate_profile_reporting(self, profile, mode, NoticeDelivery::Delivered)
            .map(StartedAgent::from)
    }

    fn create_and_activate(
        &self,
        profile: &AgentProfile,
        delivery: NoticeDelivery,
    ) -> Result<Activated, NotStarted> {
        AgentManager::create_and_activate(self, profile, delivery)
    }

    fn rename_agent(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<RenameOutcome, PtyError> {
        AgentManager::try_rename_agent(self, id, display_name)
    }

    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> Result<bool, PtyError> {
        AgentManager::try_reparent_agent(self, child, parent)
    }

    fn set_workspace(&self, id: AgentId, cwd: &str) -> Result<WorkspaceSet, WorkspaceRefusal> {
        AgentManager::try_set_workspace(self, id, cwd)
    }

    fn duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError> {
        AgentManager::try_duplicate_agent(self, id)
    }

    fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
        AgentManager::list_queued_inputs(self, id).ok()
    }

    fn cancel_queued_input(
        &self,
        id: AgentId,
        input_id: &str,
    ) -> Result<CancelOutcome, CancelError> {
        AgentManager::cancel_queued_input(self, id, input_id)
    }

    fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
        AgentManager::interrupt(self, id)
    }
}

/// The backend output format of a newly created agent.
///
/// ★Why StreamJson★: the LLM is the caller and the frontend creation command the LLM calls (`agentlist.createAgent`) already defaults to StreamJson (ADR-0078); different defaults per entry would give "created it, but the screen is different".
/// ★The home of the value is here alone★ — new create-verb entries reference this instead of their own constant.
/// ★**Only one place pins the value itself**★ — `tests::the_new_agent_default_output_format_is_pinned_to_a_literal`; all other assertions compare relatively and would stay green if the value flipped.
pub const NEW_AGENT_OUTPUT_FORMAT: CoreAgentOutputFormat = CoreAgentOutputFormat::StreamJson;

// ── Backend creation policy of the LLM control surface (ADR-0219 · ADR-0279) ───────────────────────────
//
// ★The grounds switched★ — this table was set up as 「do not open」 (TRD S21 §6-G) on the premise that **the LLM cannot get past the folder
//   trust modal**; measurement overturned it (the app-server link has no such modal) and the refusal was removed
// (canonical record: ADR-0219). Do not close the table again on the old coordinates (already rejected as (b) in ADR-0219's 「Rejected
//   alternatives」).
//
// ★This table is the policy's only home★ — the doors that ask it live in two crates:
//   ① `agent.new` (this file) — the declared vocabulary `AgentBackend` narrows on its own, and this table stands one
//      layer above it (it stops, at run time, an edit that widens the vocabulary without widening the policy — that
//      arm is not reached today).
//   ② the daemon `SpawnByCwd` arm (`engram-dashboard-daemon` `connection_core.rs`) — does not ask the table itself:
//      it hands its policy (production = [`llm_creation_refusal`]) to body ③, which asks with the declared spelling
//      before the ad-hoc profile reaches the roster or the disk (ADR-0279 · ADR-0348). The shell `agent.spawnInto`
//      and the front `slot.createAgentHere` · `agent.spawn` all send that packet.
//      ★This door does not tell callers apart — despite the name "LLM control surface", human paths see this table
//      too★ (the daemon does not know who sent the packet · ADR-0279 "accepted cost" · no effect today — the table
//      closes nothing).
//   ③ the one creation body [`create_and_start`] (this file; bus `agent.spawn --cwd`) — asked with the declared
//      spelling before anything is registered (ADR-0348 decision 4). Pinned by
//      `tests::the_body_refuses_a_closed_backend_before_calling_the_host` (the body) and
//      `tests::the_bus_cwd_arm_hands_the_production_policy_to_the_body` (the wiring).
//   ④ `agent.duplicate` (this file, [`verb_duplicate`]) — asked with the original's backend word (`launch::backend_word`,
//      lowercase — the table compares ignoring case) before the host is called, so a shell or gemini original is not
//      copied (the ADR-0279 cost, ADR-0386 decision 8).
// ★The shell is not a door★ — `agent.spawnInto` carries only a typo net (`parse_backend`, asks the wire enum) and does not depend on this
//   crate (ADR-0270 decision 1).
// ★Some create paths do not see this table — do not miss them when counting doors★ (an uncounted door stays open
// without ever asking): the daemon `CreateProfile` (the front's reserved-node create doors — `agentlist.createCodex`
// and the like — go there). The front command registry (`humanOnly`) is not a door of this table — the codex door's
// `humanOnly` was removed on 2026-09-22.
// Per-door lists would show 「a backend one door creates but another cannot」. Mismatch measured at ①
//   `tests::new_creates_exactly_what_the_llm_backend_policy_opens` · `tests::new_advertises_exactly_the_backends_the_llm_policy_opens`
//   (this file) ·
// ② the daemon's `every_wire_backend_declares_an_llm_policy` in `connection_core.rs` · `the_production_core_asks_the_agent_backend_policy`
//   · `spawn_by_cwd_refuses_a_backend_the_policy_closes`.
//
// ★Why the table is keyed by a word, not a type★ — the wire vocabulary lives in `protocol::AgentBackendKind`, which
// this crate does not depend on (the canon of that ban = the `[dependencies]` comment in this crate's `Cargo.toml`),
// so the one thing the daemon and this crate can both read is the word. The daemon maps the wire enum onto the
// declared [`AgentBackend`] (a match with no wildcard) and the doors ask with the declared spelling
// ([`backend_word`]); the table compares ignoring case. A wire word with no row in the table is caught by ②'s first
// test (`every_wire_backend_declares_an_llm_policy`).

/// One line of the policy table.
pub struct LlmBackendPolicy {
    /// The backend word, compared **case-insensitively** — the declaration vocabulary writes `Claude`, the wire vocabulary `claude` (the two enums' docs carry why).
    pub word: &'static str,
    /// `None` = this surface creates it. `Some` = it does not — carries **the reason and when it opens** together.
    pub refusal: Option<&'static str>,
}

/// ★Which backends the LLM control surface can create — the only place of that policy★. The daemon `SpawnByCwd` door applies this table without distinguishing callers and blocks human paths too (② of the section header · ADR-0279).
///
/// ★Today this table refuses nothing★ (codex's refusal was removed — reason on that line). It stays because the next backend must be 「no declaration = closed」 ([`NO_POLICY_DECLARED`]) and it keeps creation doors from each holding a list.
///
/// ★Reopen ADR-0279 before changing any line's `refusal` to `Some` or bringing in an undeclared word★ — the daemon door blocks human paths too; that edit is the trigger for adding a caller axis to the spawn packet.
/// ★Opening one line alone does not finish it★ — `agent.new` creates only after that variant is added to the `AgentBackend` vocabulary and the pair is filled in [`backend_command`]; otherwise the tests above go red.
/// ★This table splits only the backend word — the output mode axis already has an LLM expression★: `output_format` of `agent.new` (`Terminal`|`StreamJson`, unspecified = [`NEW_AGENT_OUTPUT_FORMAT`]) which [`verb_new`] passes to [`backend_command`] **regardless of
/// the backend**, so opening a word opens both modes. Do not build a second entry on the mode axis (CLAUDE.md 「LLM-first control」: do not make a second surface).
pub const LLM_BACKEND_POLICY: &[LlmBackendPolicy] = &[
    LlmBackendPolicy {
        word: "claude",
        refusal: None,
    },
    // ★codex's refusal was removed because the premise it was built on was overturned by measurement★. Old reason: 「in a never-seen folder
    //   codex shows its own trust modal and a non-human caller cannot get past it」. Measured (codex 0.155.1):
    //   the `app-server` path returns `thread/start` as **success** for a never-seen folder (~338 ms · no error · no inbound trust
    //   request) and **writes that folder itself** into its `config.toml` as `trust_level = "trusted"` — the modal was **TUI-only**.
    // ★No code checking trust is kept★ — the path has no gate, and a pretend check would only add a dependency reading codex's TOML
    //   settings while guarding nothing.
    LlmBackendPolicy {
        word: "codex",
        refusal: None,
    },
];

/// The reason for a word not in the table — ★absence is not open but closed★. If a new backend silently opened without a policy declaration,
/// this gate would have guarded nothing.
const NO_POLICY_DECLARED: &str = "no LLM control surface policy has been declared for this backend yet — it is closed until declared (add one line to `commands::LLM_BACKEND_POLICY` of engram-dashboard-agent).";

/// Whether **the door asking this policy can create** the backend of this word now — `None` = creates, `Some(reason)` = does not.
///
/// ★A different axis from 「an unknown word」★ — `Some` here is also a legitimate backend this repository knows; lumping the two refusals into one text would make the caller fix a typo that does not exist. Emit the two axes with different texts.
/// ★Do not read it as blocking only non-human callers★ — the daemon `SpawnByCwd` door applies it without distinguishing callers (frontend `slot.createAgentHere` · `agent.spawn` too — ADR-0279); only paths that do not ask (daemon `CreateProfile` etc.) create as is.
///
/// A word not in the table is also `Some` (fail-closed).
pub fn llm_creation_refusal(backend_word: &str) -> Option<&'static str> {
    LLM_BACKEND_POLICY
        .iter()
        .find(|policy| policy.word.eq_ignore_ascii_case(backend_word))
        .map_or(Some(NO_POLICY_DECLARED), |policy| policy.refusal)
}

/// The word of the declaration vocabulary — asking the policy table needs a word. `AgentBackend` has only unit variants, so serde serialization is the word itself (hand-writing would silently diverge as variants grow); a serialization failure gives an empty word and the policy closes (fail-closed).
fn backend_word(backend: &AgentBackend) -> String {
    serde_json::to_value(backend)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Declaration vocabulary → run command.
///
/// ★This `match` sweeps **the declaration enum** — core backend growth does not break compilation here★. The net for that direction (core grew, declaration stays narrow) is the reverse `match` sweeping `AgentCommand` in
/// `tests::the_declared_vocabularies_are_pinned_to_the_core_ones`.
/// ★`output_format` is carried as is **regardless of the backend**★ — opening one word opens both modes (codex's two-mode split is `backend::codex::is_app_server`, which also reads this field).
fn backend_command(backend: AgentBackend, output_format: CoreAgentOutputFormat) -> AgentCommand {
    match backend {
        AgentBackend::Claude => AgentCommand::Claude {
            extra_args: vec![],
            output_format,
        },
        // ★`extra_args` empty is a decision★ — this entry does not grow the argument surface (no model / reasoning-effort field); codex's
        //   arguments are all assembled from this command by `backend::codex::build_spec`.
        AgentBackend::Codex => AgentCommand::Codex {
            extra_args: vec![],
            output_format,
        },
    }
}

/// Declaration vocabulary → core vocabulary. **Unspecified is [`NEW_AGENT_OUTPUT_FORMAT`]**, and values outside the vocabulary are rejected
/// by deserialization as `INVALID_ARGUMENT` before reaching here (no silent fallback — the same discipline as the frontend's `coerceOutputFormat`).
fn output_format(given: Option<AgentOutputFormat>) -> CoreAgentOutputFormat {
    match given {
        None => NEW_AGENT_OUTPUT_FORMAT,
        Some(AgentOutputFormat::Terminal) => CoreAgentOutputFormat::Terminal,
        Some(AgentOutputFormat::StreamJson) => CoreAgentOutputFormat::StreamJson,
    }
}

/// Assembles the `agent.*` table — ★the only place real handlers come in★ (rule T-1).
///
/// ★blocking contract★: handler bodies write to disk holding the profile lock and poll for resume early exit (`AgentManager::activate_profile`); polling on an async runtime thread would block it, so the assembly must call it behind `spawn_blocking` — both
///   surfaces (the daemon's `/control/agent` route and command bus delivery) come through that one common entry, which sits on the blocking pool.
/// ★`notify` is an argument★: a roster-changing verb must notify (port doc) but the real thing is the daemon's; passing it makes it **impossible to omit** (a trait default would silently not be called).
/// ★`usage` is an argument for the same reason★: the daemon owns the real `usage.*`; assemblies outside production (smoke bins · tests) pass a port without fields, where `usage.*` answers `NOT_FOUND`.
/// ★`library` likewise★: production passes asset-library's service; other assemblies pass a port that answers
///   `library-unavailable` (daemon `NoLibrary`).
/// Command growth changes the declaration block and one line here, not the assembly (the executable).
// ADR-0155
// ADR-0386
pub fn make_table(
    host: Arc<dyn AgentCommandHost>,
    notify: Arc<dyn RosterChanged>,
    usage: Arc<dyn UsageCommandHost>,
    library: Arc<dyn LibraryCommandHost>,
) -> CommandTable {
    let mut table = CommandTable::new(COMMAND_SPECS);

    let list = Arc::clone(&host);
    let resolve = Arc::clone(&host);
    let spawn = (Arc::clone(&host), Arc::clone(&notify));
    let new = (Arc::clone(&host), Arc::clone(&notify), Arc::clone(&library));
    let rename = (Arc::clone(&host), Arc::clone(&notify));
    let move_ = (Arc::clone(&host), Arc::clone(&notify));
    let set_workspace = (Arc::clone(&host), Arc::clone(&notify));
    let duplicate = (Arc::clone(&host), Arc::clone(&notify));
    let list_queued = Arc::clone(&host);
    let cancel_queued = Arc::clone(&host);
    let interrupt = Arc::clone(&host);
    let usage_get = Arc::clone(&usage);
    let usage_refresh = usage;

    // ★Blow up at assembly★: the three insert rejects (name outside the declared set · duplicate insertion · declared schema text not
    //   JSON) are **build-decided values**, never runtime-varying; the `TableError` carried with the panic tells which, so messages below
    //   do not assert the reason.
    table
        .insert(
            "agent.list",
            blocking_handler(move |_: AgentListArgs| verb_list(list.as_ref())),
        )
        .expect("could not plug agent.list into the table");
    table
        .insert(
            "agent.resolve",
            blocking_handler(move |args: AgentResolveArgs| verb_resolve(resolve.as_ref(), args)),
        )
        .expect("could not plug agent.resolve into the table");
    table
        .insert(
            "agent.spawn",
            blocking_handler(move |args: AgentSpawnArgs| {
                verb_spawn(spawn.0.as_ref(), spawn.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.spawn into the table");
    table
        .insert(
            "agent.new",
            blocking_handler(move |args: AgentNewArgs| {
                verb_new(new.0.as_ref(), new.1.as_ref(), new.2.as_ref(), args)
            }),
        )
        .expect("could not plug agent.new into the table");
    table
        .insert(
            "agent.rename",
            blocking_handler(move |args: AgentRenameArgs| {
                verb_rename(rename.0.as_ref(), rename.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.rename into the table");
    table
        .insert(
            "agent.move",
            blocking_handler(move |args: AgentMoveArgs| {
                verb_move(move_.0.as_ref(), move_.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.move into the table");
    // ADR-0386: blocking — the folder stat waits on bounded IO (up to the read budget).
    table
        .insert(
            "agent.setWorkspace",
            blocking_handler(move |args: AgentSetWorkspaceArgs| {
                verb_set_workspace(set_workspace.0.as_ref(), set_workspace.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.setWorkspace into the table");
    table
        .insert(
            "agent.duplicate",
            blocking_handler(move |args: AgentDuplicateArgs| {
                verb_duplicate(duplicate.0.as_ref(), duplicate.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.duplicate into the table");
    table
        .insert(
            "agent.listQueuedInputs",
            blocking_handler(move |args: AgentListQueuedInputsArgs| {
                verb_list_queued_inputs(list_queued.as_ref(), args)
            }),
        )
        .expect("could not plug agent.listQueuedInputs into the table");
    table
        .insert(
            "agent.cancelQueuedInput",
            blocking_handler(move |args: AgentCancelQueuedInputArgs| {
                verb_cancel_queued_input(cancel_queued.as_ref(), args)
            }),
        )
        .expect("could not plug agent.cancelQueuedInput into the table");
    table
        .insert(
            "agent.interrupt",
            blocking_handler(move |args: AgentInterruptArgs| {
                verb_interrupt(interrupt.as_ref(), args)
            }),
        )
        .expect("could not plug agent.interrupt into the table");
    // ★Both are `blocking_handler` (TRD §3 #17)★ — the bodies wait for the query to end, but as async the daemon entry would cut them off
    //   at the first
    //   poll with `OUTCOME_UNKNOWN`. The waiting ends inside the port (a std channel).
    table
        .insert(
            "usage.get",
            blocking_handler(move |args: UsageGetArgs| verb_usage_get(usage_get.as_ref(), args)),
        )
        .expect("could not plug usage.get into the table");
    table
        .insert(
            "usage.refresh",
            blocking_handler(move |args: UsageRefreshArgs| {
                verb_usage_refresh(usage_refresh.as_ref(), args)
            }),
        )
        .expect("could not plug usage.refresh into the table");
    crate::library_commands::insert(&mut table, &host, library);

    table
}

fn verb_usage_get(
    usage: &dyn UsageCommandHost,
    args: UsageGetArgs,
) -> Result<UsageVendorRow, CommandError> {
    usage
        .get(&args.backend)
        .ok_or_else(|| no_usage_limits(&args.backend))
}

fn verb_usage_refresh(
    usage: &dyn UsageCommandHost,
    args: UsageRefreshArgs,
) -> Result<UsageRefreshOk, CommandError> {
    usage
        .refresh(&args.backend)
        .map(UsageRefreshOk::from)
        .ok_or_else(|| no_usage_limits(&args.backend))
}

/// A backend without a field — **the same code and the same sentence** as the same absence of wire ⟳ (`no_usage_text` of daemon
/// `connection_core`). Only the word is carried in this entry's spelling (`Claude`).
fn no_usage_limits(backend: &AgentBackend) -> CommandError {
    CommandError::not_found(format!(
        "this daemon keeps no usage limits for backend '{}'",
        backend_word(backend)
    ))
}

fn verb_list(host: &dyn AgentCommandHost) -> Result<AgentListOk, CommandError> {
    // ★Everything is built from one roster lookup★: name · liveness · cwd · hierarchy must come from the same snapshot for the rows within
    //   one response
    //   to be consistent.
    let agents = host
        .roster()
        .into_iter()
        .map(|row| AgentRow {
            id: row.id.to_string(),
            name: row.canonical_name,
            state: state_of(row.live.as_ref()).to_string(),
            cwd: row.cwd,
            parent: row.parent.map(|p| p.to_string()),
        })
        .collect();
    // Not a value at the same moment as the roster sheet — the store state is as of the last save attempt, so it only matches that well to
    //   begin with.
    let store = host.store_status();
    Ok(AgentListOk {
        agents,
        store: RosterStoreRow {
            agents: store_status_row(store),
        },
    })
}

// The cap of TRD S28 §6.
const RESOLVE_TARGETS_MAX: usize = 8;

// ADR-0307
fn verb_resolve(
    host: &dyn AgentCommandHost,
    args: AgentResolveArgs,
) -> Result<AgentResolveOk, CommandError> {
    let count = args.targets.len();
    if !(1..=RESOLVE_TARGETS_MAX).contains(&count) {
        return Err(CommandError::invalid_argument(format!(
            "targets takes 1 to {RESOLVE_TARGETS_MAX} agent names or ids, got {count}"
        )));
    }
    // Blank words are all gathered and emitted at once — the head is only the first position, so the rest are appended at the end of the text.
    let blanks: Vec<usize> = args
        .targets
        .iter()
        .enumerate()
        .filter(|(_, target)| is_blank(Some(target)))
        .map(|(index, _)| index)
        .collect();
    if let Some((&first, rest)) = blanks.split_first() {
        let mut message = "blank value — an empty argument is usually an unset shell variable; every target needs an agent name or id".to_string();
        if !rest.is_empty() {
            let others: Vec<String> = rest
                .iter()
                .map(|index| format!("targets[{index}]"))
                .collect();
            message.push_str(&format!(" (also blank: {})", others.join(", ")));
        }
        return Err(target_failure(
            first,
            CommandError::invalid_argument(message),
        ));
    }
    // ★Resolve everything with one roster sheet★ — reading again per word would have the lines within one answer see the roster at
    //   different moments.
    let roster = host.roster();
    let mut agents = Vec::with_capacity(count);
    for (index, target) in args.targets.into_iter().enumerate() {
        let resolved =
            resolve_in(&roster, &target).map_err(|error| target_failure(index, error))?;
        let live = roster
            .iter()
            .find(|row| row.id == resolved.id)
            .is_some_and(|row| state_of(row.live.as_ref()) == AGENT_STATE_LIVE);
        agents.push(ResolvedAgentRow {
            target,
            id: resolved.id.to_string(),
            name: resolved.name,
            live,
        });
    }
    Ok(AgentResolveOk { agents })
}

/// Puts the head `targets[i]: ` of that position on a failure caused by one word — the code and retry are left as is.
// ADR-0307
fn target_failure(index: usize, mut error: CommandError) -> CommandError {
    let message = format!("targets[{index}]: {}", error.message());
    error.set_message(message);
    error
}

/// Spreads the store state (a tag enum carrying a reason) into a word + optional fields.
// ADR-0291 R17 (D6)
fn store_status_row(status: StoreStatus) -> StoreStatusRow {
    let (state, refusal) = match status {
        StoreStatus::Writable => (StoreStateWord::Writable, None),
        StoreStatus::ReadOnly(refusal) => (StoreStateWord::ReadOnly, Some(refusal)),
        StoreStatus::Refusing(refusal) => (StoreStateWord::Refusing, Some(refusal)),
    };
    StoreStatusRow {
        state,
        reason: refusal.map(|refusal| match refusal {
            Refusal::Newer { .. } => StoreRefusalWord::Newer,
            Refusal::Unreadable => StoreRefusalWord::Unreadable,
        }),
        file_version: match refusal {
            Some(Refusal::Newer { found }) => Some(found),
            _ => None,
        },
    }
}

fn verb_spawn(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentSpawnArgs,
) -> Result<AgentSpawnOk, CommandError> {
    let (target, cwd, name) = (
        args.target.as_deref(),
        args.cwd.as_deref(),
        args.name.as_deref(),
    );
    // ★The constraint of this verb is **a combination**, not one field★: all three are `Option` by declaration but a call needs `target`
    //   or `cwd`. 「May be omitted」 guidance would get the omitting retry **rejected again** with 「one of the two must be given」; a text
    //   looking at one field cannot state this, so it is written here.
    let blanks = blank_fields(&[("target", target), ("cwd", cwd), ("name", name)]);
    if !blanks.is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "blank value(s) for: {} — an empty argument is usually an unset shell variable. spawn needs exactly one of target (an existing agent to wake) or cwd (a folder to create one in); name only applies when creating",
            blanks.join(", ")
        )));
    }

    // ★Two verbs use one name (wake / create and launch)★: which one is decided by the arguments. If both are given
    //   there is no ground to choose, and silently choosing one would wake someone else instead of the agent meant to be created.
    match (target, cwd) {
        (Some(_), Some(_)) => Err(CommandError::invalid_argument(
            "spawn takes either an existing agent (target) or cwd for a new one, not both",
        )),
        (None, None) => Err(CommandError::invalid_argument(
            "spawn needs either an existing agent to wake (target) or cwd to create a new one",
        )),
        // Waking does not change the name, so a name does nothing — silently ignoring it would make the caller think the name changed.
        // ★No command identifier in the text★: `agent.rename` is a catalog name, not a typeable command, and writing it would make the LLM
        //   type it into the shell; the adapter attaches per-surface notation.
        (Some(token), None) if name.is_some() => Err(CommandError::invalid_argument(format!(
            "name does not apply when waking an existing agent ({token}) — changing a name is a separate verb, so drop this field"
        ))),
        (Some(token), None) => wake_existing(host, notify, token),
        // Claude fixed on purpose — `agent.new` is the one bus door that picks a backend; moves with front `agent.spawn`.
        // ADR-0348
        (None, Some(cwd)) => create_and_start(
            host,
            notify,
            NewAgent {
                cwd: cwd.to_string(),
                name: name.map(str::to_string),
                backend: AgentBackend::Claude,
                policy: llm_creation_refusal,
                delivery: NoticeDelivery::Delivered,
            },
        )
        .map(|activated| started_payload(activated.into(), true))
        .map_err(|refused| {
            spawn_create_refusal(
                refused,
                &normalize_spelling(cwd),
                &backend_word(&AgentBackend::Claude),
            )
        }),
    }
}

/// ★Waking also emits a roster notification — even with entry count, names and hierarchy unchanged★: the activation outcome changes that entry's 「last failure」 (success = clear / failure = record, ADR-0172), an axis that flows **only through the profile list**.
/// The manager's liveness transitions (`agent_list_updated`) are the live session list and do not carry it — unannounced, an LLM wake-up result would never appear on screen (the human click path does; CLAUDE.md 「LLM-first control」).
// ADR-0172
fn wake_existing(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    token: &str,
) -> Result<AgentSpawnOk, CommandError> {
    let id = resolve(host, token)?.id;
    let Some(profile) = host.agent_snapshot(id) else {
        return Err(not_found(token));
    };
    // ★The mode derivation rule is the same as the WS path (ADR-0076)★: resume if there is a stored session, otherwise new — a different
    //   rule would make one agent lose its history depending on which entry wakes it.
    //   ★Not split by sid presence alone (ADR-0185)★: first ask the backend whether the stored sid is **resumable with that command** —
    //   owned by the same dispatch as boot restore and the WS entry; otherwise a non-resumable shape would launch as Resume,
    //     opening a new conversation reported as 「resumed」.
    let mode = if crate::backend::can_resume_profile(&profile) {
        SpawnMode::Resume
    } else {
        SpawnMode::Fresh
    };
    let started = host.activate_profile(&profile, mode);
    // ★It announces on failure too — even on a save refusal★: an activation failure writes that entry's 「last failure」 (ADR-0172 ·
    //   ADR-0291 E2 — this notification was excluded from 「side effects after a change only on `Ok`」).
    notify.roster_changed();
    let started = started.map_err(|e| match e {
        PtyError::Store(error) => store_error(
            "start",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not start agent '{token}': {}",
                    store_error_reason(PROFILES_FILE, &error)
                ),
            ),
        ),
        // A launch refused before any process is a `CONFLICT` so the daemon releases the request id
        //   (`command_delivery::retains_the_id`) — `INTERNAL` would hold it and swallow the fixed retry as
        //   `ALREADY_APPLIED` (TRD S25 §1-5 (ga) 1 ①).
        PtyError::LaunchBlocked(blocked) => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "{LAUNCH_REFUSAL} for agent '{token}': {}; nothing was started — fix the listed items, then start it again",
                launch_blocked_reason(&blocked)
            ),
        )
        .with_args(blocked.to_args()),
        PtyError::LaunchRaced => CommandError::with_retry(
            ErrorCode::Conflict,
            format!(
                "{LAUNCH_REFUSAL} for agent '{token}': {LAUNCH_RACED_CAUSE}; nothing was started — try again"
            ),
            RetryMode::AfterCondition,
        ),
        // ADR-0386: lost the reservation to `agent.setWorkspace` — retryable, so not `INTERNAL` (which keeps the id).
        PtyError::WorkingFolderChanging => CommandError::with_retry(
            ErrorCode::Conflict,
            format!(
                "{LAUNCH_REFUSAL} for agent '{token}': {WORKING_FOLDER_CHANGING_CAUSE}; nothing was started — start it again once that change is done"
            ),
            RetryMode::AfterCondition,
        ),
        other => CommandError::internal(format!("could not start agent '{token}': {other}")),
    })?;
    Ok(started_payload(started, false))
}

/// A new agent to create in a folder and start — the input of [`create_and_start`].
// ADR-0348
pub struct NewAgent {
    /// As the caller typed it — the body normalizes the spelling.
    pub cwd: String,
    /// The requested name; the registered one may differ (roster uniqueness suffix).
    pub name: Option<String>,
    pub backend: AgentBackend,
    /// `None` = create · `Some(reason)` = refuse. Asked with the declared spelling (`"Claude"`, `"Codex"`).
    /// Production = [`llm_creation_refusal`].
    pub policy: fn(&str) -> Option<&'static str>,
    /// Whether the caller shows the success reply's notices to a reader.
    pub delivery: NoticeDelivery,
}

/// Why [`create_and_start`] did not answer with a started agent.
// ADR-0348
#[derive(Debug)]
pub enum CreateRefused {
    /// The backend policy refused — nothing registered, no last failure, the host was not called.
    Policy { reason: &'static str },
    /// Refused or failed with nothing registered now. In practice: a store refusal or write failure, the roster
    /// ceiling, name space, or a launch block before the add (the pre-create check returns only `LaunchBlocked`); also
    /// a failure after the add whose row a concurrent delete removed before the `created` probe (TRD §5).
    NotCreated(PtyError),
    /// The agent was registered and is asleep under `id` / `name`; `error` is why it did not start.
    CreatedAsleep {
        id: AgentId,
        name: String,
        error: PtyError,
    },
}

/// ★The one creation body★ — every entrance that creates a new agent for a folder and starts it calls this; do not
/// add a second body. Policy, spelling, the default launch command, the roster notify and the created/not-created
/// split live here; the pre-create launch check, link verdict, last-failure record and notice delivery come from
/// the host's Fresh activation path ([`AgentCommandHost::create_and_activate`]).
///
/// Notifies the roster once, after the activation, exactly when the agent was registered (success or
/// [`CreateRefused::CreatedAsleep`]) — never for a refusal that left the roster as it was.
/// Blocking — call it off async workers.
// ADR-0348
pub fn create_and_start(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    req: NewAgent,
) -> Result<Activated, CreateRefused> {
    if let Some(reason) = (req.policy)(&backend_word(&req.backend)) {
        return Err(CreateRefused::Policy { reason });
    }
    let profile = new_profile(
        &req.cwd,
        req.name,
        backend_command(req.backend, NEW_AGENT_OUTPUT_FORMAT),
    );
    let id = profile.id;
    match host.create_and_activate(&profile, req.delivery) {
        Ok(activated) => {
            notify.roster_changed();
            Ok(activated)
        }
        Err(NotStarted {
            error,
            created: false,
        }) => Err(CreateRefused::NotCreated(error)),
        // Announced even though it failed: the asleep row and its last failure reach the screen only through the
        //   roster frames (ADR-0172).
        Err(NotStarted {
            error,
            created: true,
        }) => {
            notify.roster_changed();
            let name = host
                .agent_snapshot(id)
                .map_or_else(|| id.to_string(), |p| p.canonical_name_when_live());
            Err(CreateRefused::CreatedAsleep { id, name, error })
        }
    }
}

/// The text of a create that registered the agent but did not start it:
/// `agent '<name>' (<id>) was created but did not start: <cause> — it is registered and asleep, so <next> instead of
/// creating another`. `<cause>` carries no instruction of its own (a "try again" reads as "create again"); `<next>`
/// says to start that agent by name — after fixing the listed items for a block, which does not clear by waiting.
// ADR-0348
pub fn created_but_not_started(name: &str, id: AgentId, error: &PtyError) -> String {
    const AGAIN: &str = "start it again by that name (in a moment)";
    let (cause, next) = match error {
        PtyError::Store(store) => (store_error_cause(PROFILES_FILE, store), AGAIN),
        PtyError::LaunchBlocked(blocked) => (
            launch_blocked_reason(blocked),
            "fix the listed items first, then start it by that name",
        ),
        // The cause only — `PtyError::LaunchRaced`'s text ends in "try again".
        PtyError::LaunchRaced => (LAUNCH_RACED_CAUSE.to_string(), AGAIN),
        other => (other.to_string(), AGAIN),
    };
    format!(
        "agent '{name}' ({id}) was created but did not start: {cause} — it is registered and asleep, so {next} instead of creating another"
    )
}

/// The bus answer of a [`create_and_start`] refusal. `cwd` names the folder in a block's text; `word` names the
/// backend in the policy text.
// Every failure after the add answers INTERNAL with `args.agent_id`: the agent stays registered, and a CONFLICT would
//   release the request id so a same-id retry creates an undeletable duplicate. A refusal before the add is a
//   CONFLICT so the fixed retry is not swallowed (`command_delivery::retains_the_id`).
// ADR-0122
// ADR-0348
fn spawn_create_refusal(refused: CreateRefused, cwd: &str, word: &str) -> CommandError {
    match refused {
        CreateRefused::Policy { reason } => policy_refusal(word, reason),
        CreateRefused::NotCreated(PtyError::LaunchBlocked(blocked)) => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "{LAUNCH_REFUSAL} for a new agent in '{cwd}': {}; nothing was created — fix the listed items, then try again",
                launch_blocked_reason(&blocked)
            ),
        )
        .with_args(blocked.to_args()),
        // The pre-create check never races (it returns only a block); this arm is reached only when a concurrent
        //   delete removed a row that raced after the add — "nothing was created" is then true of the end state.
        CreateRefused::NotCreated(PtyError::LaunchRaced) => CommandError::with_retry(
            ErrorCode::Conflict,
            format!(
                "{LAUNCH_REFUSAL} for a new agent in '{cwd}': {LAUNCH_RACED_CAUSE}; nothing was created — try again"
            ),
            RetryMode::AfterCondition,
        ),
        CreateRefused::NotCreated(other) => create_error(other, cwd),
        CreateRefused::CreatedAsleep { id, name, error } => {
            let text = created_but_not_started(&name, id, &error);
            let (failure, mut args) = match error {
                // ADR-0291 R17
                PtyError::Store(store) => (
                    store_error("start", &store, CommandError::internal(text)),
                    serde_json::Map::new(),
                ),
                PtyError::LaunchBlocked(blocked) => {
                    (CommandError::internal(text), blocked.to_args())
                }
                _ => (CommandError::internal(text), serde_json::Map::new()),
            };
            args.insert("agent_id".into(), id.to_string().into());
            failure.with_args(args)
        }
    }
}

/// A backend the policy closes — the same words on every door that asks it.
fn policy_refusal(word: &str, reason: &str) -> CommandError {
    CommandError::invalid_argument(format!(
        "backend '{word}' is a known word, but this surface does not create it now — {reason}"
    ))
}

fn verb_new(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    library: &dyn LibraryCommandHost,
    args: AgentNewArgs,
) -> Result<AgentNewOk, CommandError> {
    let (cwd, name) = (args.cwd.as_str(), args.name.as_deref());
    reject_blanks(&[
        ("cwd", Some(cwd), Blank::NeedsValue),
        ("name", name, Blank::OrOmit),
    ])?;
    // ★No default★ — unspecified is rejected by deserialization (`AgentNewArgs::backend` doc). Do not bring back `unwrap_or(Claude)`: with
    //   two backends it silently folds 「did not choose」 into claude (already removed from the spawn packet for the same reason).
    let backend = args.backend.clone();
    let word = backend_word(&backend);
    // ★Every door asking the policy looks at this one table★ — this `if` runs on every call but **the refusal arm does not reach today**:
    //   the vocabulary and the table open exactly the same set (claude · codex) and **the table closes no word**; words outside the table
    //   are rejected
    //   earlier by deserialization (ADR-0219).
    //   ★Do not delete it★ — it blocks 「widening the vocabulary without widening [`LLM_BACKEND_POLICY`]」 and comes alive the day that edit
    //   arrives; `tests::new_creates_exactly_what_the_llm_backend_policy_opens` measures that (red if the vocabulary is widened without
    //   opening the
    //   table). Sibling door: the one creation body [`create_and_start`] (called by the daemon `SpawnByCwd` · the bus `agent.spawn --cwd` —
    //   does not distinguish callers · ADR-0279 · ADR-0348), which also blocks nothing by policy today. The shell `agent.spawnInto` is not a door (its refusal is the `parse_backend` typo net, ADR-0270);
    //   the frontend `commands/registry.ts` is not a sibling door.
    if let Some(reason) = llm_creation_refusal(&word) {
        return Err(policy_refusal(&word, reason));
    }
    let profile = new_profile(
        cwd,
        name.map(str::to_string),
        backend_command(backend, output_format(args.output_format)),
    );
    // ADR-0386: judged before the create under one library read lock, released before `create_agent` takes the
    //   name-allocation gate; the profile is created with the accepted list in one registry write — no profile-edit
    //   port step after it, so no failure can follow the create. Without `presets` nothing is judged and
    //   `root_generation` is not read.
    // ADR-0400 decision 18: the registry insert inserts only while the library's change number is still the one the
    //   judgement was made at; on a change nothing is inserted, every lock is released, and the request — its paths
    //   as given — is judged again. It ends once no library change lands between a judgement and its insert.
    let (stored, skipped) = loop {
        let Some(presets) = &args.presets else {
            break (register(host, profile)?, Vec::new());
        };
        let judged = library
            .judge_new_presets(&profile, presets, args.root_generation)
            .map_err(|refusal| refusal_error("agent.new", refusal))?;
        let mut judged_profile = profile.clone();
        judged_profile.default_preset = judged.attachments.default_preset;
        judged_profile.presets = judged.attachments.presets;
        let Some(at) = judged.change else {
            break (register(host, judged_profile)?, judged.skipped);
        };
        let still = || library.change_number() == Some(at);
        match register_if(host, judged_profile, &still)? {
            Some(stored) => break (stored, judged.skipped),
            None => tracing::debug!(
                judged_at = at,
                "agent.new: the library changed between the judgement and the insert; judging again"
            ),
        }
    };
    notify.roster_changed();
    Ok(AgentNewOk {
        agent_id: stored.id.to_string(),
        name: stored.canonical_name_when_live(),
        state: AGENT_STATE_SLEEPING.to_string(),
        skipped,
    })
}

/// The registration of `agent.new` (create without starting).
///
/// ★The settled name can differ from the requested one★: a suffix is attached for global roster uniqueness (ADR-0120/0123), so the response reads the name from **the registered value** — the requested name would diverge from the screen and address.
fn register(
    host: &dyn AgentCommandHost,
    profile: AgentProfile,
) -> Result<AgentProfile, CommandError> {
    let cwd = profile.cwd.to_string_lossy().into_owned();
    host.create_agent(profile)
        .map_err(|e| create_error(e, &cwd))
}

/// [`register`] whose insert inserts only while `still` answers `true` ([`AgentCommandHost::create_agent_if`]);
/// `Ok(None)` = nothing registered.
// ADR-0400 decision 18
fn register_if(
    host: &dyn AgentCommandHost,
    profile: AgentProfile,
    still: &dyn Fn() -> bool,
) -> Result<Option<AgentProfile>, CommandError> {
    let cwd = profile.cwd.to_string_lossy().into_owned();
    host.create_agent_if(profile, still)
        .map_err(|e| create_error(e, &cwd))
}

/// A new agent's profile for the folder the caller typed — both create doors (`agent.new` · the one creation body)
/// build it here.
// ADR-0348
fn new_profile(cwd: &str, name: Option<String>, command: AgentCommand) -> AgentProfile {
    // ★The typed spelling is settled here★: people and LLMs fill this field, and Windows paths come as
    //   `C:\work\thing`; values from a shell sometimes arrive still wrapped in `"…"` because of spaces. Absorb both
    //   here, or an agent with a garbled folder name sits in the roster with no verb to remove it (ADR-0122).
    //   Only the spelling is chosen here — whether the folder exists is not checked. For a stdio start a missing
    //   folder fails the process spawn (a failure after the add · ADR-0348); a PTY profile (`agent.new` can build
    //   one) has been observed to start with a missing folder — where that process then runs is unverified.
    //   An empty quote pair is not stripped, so `reject_blanks` still holds.
    let cwd = normalize_spelling(cwd);
    let mut profile = AgentProfile::new(cwd.clone(), command, PathBuf::from(&cwd), vec![], false);
    profile.display_name = name;
    profile
}

/// A create refused or failed with nothing registered. `cwd` = the normalized folder, for the
/// out-of-workspace text. Shared by `agent.new` and the bus `agent.spawn --cwd` refusal.
// ADR-0348
fn create_error(e: PtyError, cwd: &str) -> CommandError {
    // ★The caller has different things to do per failure reason★ — free a slot (CONFLICT) · fix the arguments
    //   (INVALID_ARGUMENT) · retrying is useless (INTERNAL). Mashing them into one code would lose that split.
    match e {
        // ★The text reveals the **nature** of the cap★: without this line, the first reaction of a caller (person · LLM) hitting the cap
        //   would be
        //   "raise the number" — that would be removing by oneself the backstop placed to stop runaway creation loops.
        PtyError::RosterFull { current, limit } => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "the team already has {current} agents, which is the safety ceiling ({limit}) that stops a runaway create loop — it is not a product limit and raising it is not the fix. Remove agents you no longer need, then try again"
            ),
        ),
        PtyError::CwdDenied => CommandError::invalid_argument(format!(
            "cwd is outside the allowed workspace: {cwd}"
        )),
        PtyError::Unsupported(reason) => CommandError::of(
            ErrorCode::Conflict,
            format!("could not register a new agent: {reason}"),
        ),
        PtyError::Store(error) => store_error(
            "register",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not register a new agent: {} — nothing was registered",
                    store_error_reason(PROFILES_FILE, &error)
                ),
            ),
        ),
        other => CommandError::internal(format!("could not register a new agent: {other}")),
    }
}

fn verb_rename(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentRenameArgs,
) -> Result<AgentRenameOk, CommandError> {
    let (token, name) = (args.target.as_str(), args.name.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("name", Some(name), Blank::NeedsValue),
    ])?;
    let id = resolve(host, token)?.id;
    // ★A rename the store did not accept is an error without notification★ — the name is unchanged, so there is no change to announce
    //   (ADR-0291 D3).
    let outcome = host
        .rename_agent(id, Some(name.to_string()))
        .map_err(|e| match e {
            PtyError::Store(error) => store_error(
                "rename",
                &error,
                store_command_error(
                    &error,
                    format!(
                        "could not rename '{token}': {} — the name was left as it is",
                        store_error_reason(PROFILES_FILE, &error)
                    ),
                ),
            ),
            other => CommandError::internal(format!("could not rename '{token}': {other}")),
        })?;
    // ★The four outcomes are not mashed★: the settled name · no change since it already holds that family · absent · name space exhausted.
    //   The first two are successes but different facts, so they are carried split by outcome.
    match outcome {
        RenameOutcome::Renamed(committed) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: committed,
                outcome: RENAME_OUTCOME_RENAMED.to_string(),
            })
        }
        RenameOutcome::Unchanged(kept) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: kept,
                outcome: RENAME_OUTCOME_UNCHANGED.to_string(),
            })
        }
        RenameOutcome::NotFound => Err(not_found(token)),
        RenameOutcome::Exhausted => Err(CommandError::of(
            ErrorCode::Conflict,
            format!("every numbered variant of that name is taken, so '{token}' was left as it is"),
        )),
    }
}

fn verb_move(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentMoveArgs,
) -> Result<AgentMoveOk, CommandError> {
    let token = args.target.as_str();
    // ★Absence is not 「no parent given, so detach」★ — detaching to root is the instruction `null`; folding absence would let one typo
    //   field run as a hierarchy release.
    // ★Absence over the wire cannot get here★ — the declaration macro puts no `#[serde(default)]` on this field, so deserialization
    //   rejects it with `missing field`; this branch is for code building the arguments directly.
    let Some(parent_token) = args.parent.as_ref() else {
        return Err(CommandError::invalid_argument(
            "move needs parent: a name/id to move under, or null to move it back to the top level",
        ));
    };
    let parent_token = parent_token.as_deref();
    // ★The recovery path of `parent` differs from other fields★ — answering a blank value only with 「fill in a value」 would hide, in this
    //   branch alone, the `null` branch (detach to top level) the absence text tells about. Rejecting only this field first would **hide
    //   the sibling**:
    //   with both blank the caller would hear about `target` only on the next trip. Both demands stand in the same check, different
    //   guidance per field, at once.
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        (
            "parent",
            parent_token,
            Blank::OrElse("a name/id to move under, or null to move it back to the top level"),
        ),
    ])?;
    let child = resolve(host, token)?;
    let parent = match parent_token {
        None => None,
        Some(p) => Some(resolve(host, p)?.id),
    };
    // ★A move the store did not accept is an error without notification★ — the hierarchy is unchanged, so there is no change to announce
    //   (ADR-0291 D3).
    let moved = host.reparent_agent(child.id, parent).map_err(|e| match e {
        PtyError::Store(error) => store_error(
            "move",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not move '{token}': {} — it was left where it was",
                    store_error_reason(PROFILES_FILE, &error)
                ),
            ),
        ),
        other => CommandError::internal(format!("could not move '{token}': {other}")),
    })?;
    if !moved {
        // ★`false` alone does not tell the reason★: the resolved target may have vanished in between (NOT_FOUND) or the tree structure
        //   refused it (CONFLICT) — different remedies, so the roster is looked at again (the reason list corresponds line by line to the
        //   refusal
        //   conditions of `ProfileRegistry::try_reparent`).
        // ★Name the side that vanished★: naming the child whichever vanished would, when only the parent vanished, tell the caller an
        //   agent that **is there** is missing.
        let roster = host.roster();
        let gone = |id: AgentId| !roster.iter().any(|row| row.id == id);
        if gone(child.id) {
            return Err(not_found(token));
        }
        // If parent is Some, parent_token is Some too (the match above makes the two together).
        if let Some((parent_token, parent_id)) = parent_token.zip(parent) {
            if gone(parent_id) {
                return Err(not_found(parent_token));
            }
        }
        return Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "'{token}' cannot go there. The tree is one level deep, so: an agent cannot be its own parent; the new parent must itself be top-level; and the agent being moved must not already have children."
            ),
        ));
    }
    notify.roster_changed();
    // ★This response is **not** a one-moment snapshot (a scope left knowingly)★: the name is read from the roster after applying and
    //   `parent` is **the value this call instructed**, so a concurrent move can make 「current name + the parent this call attached」 a
    //   combination
    //   never simultaneously true — still **not false**: the requested move was applied, which is all this answer states. An atomic
    //   snapshot would need an API where the manager returns the post-apply state — not decided here. Likewise **a target that vanished
    //   after applying is
    //   also `Ok`**.
    Ok(AgentMoveOk {
        agent_id: child.id.to_string(),
        // ★The name is read again from the roster **after** applying★: the resolve-time value could, after a rename committed in between,
        //   make the response 「old name + new parent」, **a combination that never existed** (the caller types the next command with that name).
        //   Cost: one roster lookup on the success path.
        // ★If unreadable, fall back to the resolve-time name — not an empty string★: a nameless row as a success response would be read by
        //   the caller's decider as "an unreadable response".
        name: current_name(host, child.id).unwrap_or(child.name),
        parent: parent.map(|p| p.to_string()),
    })
}

/// `agent.setWorkspace`'s refusal codes — args `{cwd, why}` and `{reason}`.
// ADR-0386
pub const WORKSPACE_NOT_FOUND: &str = "workspace-not-found";
pub const WORKSPACE_LOCKED: &str = "workspace-locked";
/// The `workspace-locked` reason when another working-folder change holds the agent.
pub const WORKSPACE_CHANGING: &str = "changing";

// ADR-0321
// ADR-0386
fn verb_set_workspace(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentSetWorkspaceArgs,
) -> Result<AgentSetWorkspaceOk, CommandError> {
    let (token, cwd) = (args.target.as_str(), args.cwd.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("cwd", Some(cwd), Blank::NeedsValue),
    ])?;
    let id = resolve(host, token)?.id;
    // A refusal changed nothing, so there is nothing to announce.
    let set = host
        .set_workspace(id, cwd)
        .map_err(|refusal| workspace_refusal(token, refusal))?;
    // ADR-0386: the tree shows the folder.
    notify.roster_changed();
    Ok(AgentSetWorkspaceOk {
        agent_id: id.to_string(),
        name: set.name,
        cwd: set.cwd,
    })
}

/// A `agent.setWorkspace` refusal as a command error — the coded ones in the `{code, args}` shape every coded refusal
/// carries ([`coded_error`]), an agent gone as the resolver's `NOT_FOUND`. A refusal that clears by itself — a stat
/// that timed out, a share not reachable, a launch preparing, another change — carries the retry directive; one only
/// a different call or a different state fixes (not absolute, missing, running, conversed) does not.
// ADR-0386 decision 14: the clearable ones answer `AfterCondition`, as the library's do.
// ADR-0387
fn workspace_refusal(token: &str, refusal: WorkspaceRefusal) -> CommandError {
    const COMMAND: &str = "agent.setWorkspace";
    let coded = |class: PortClass, code: &str, args: serde_json::Value, clearable: bool| {
        let reason = Reason {
            code: code.to_owned(),
            args,
            line: None,
        };
        coded_error(COMMAND, class, &reason, clearable)
    };
    match refusal {
        WorkspaceRefusal::FolderNotFound { cwd, why } => {
            let clearable =
                why == crate::launch::STAT_TIMED_OUT || why == crate::launch::SHARE_NOT_REACHABLE;
            coded(
                PortClass::InvalidArgument,
                WORKSPACE_NOT_FOUND,
                serde_json::json!({ "cwd": cwd, "why": why }),
                clearable,
            )
        }
        WorkspaceRefusal::Locked(lock) => coded(
            PortClass::Conflict,
            WORKSPACE_LOCKED,
            serde_json::json!({ "reason": lock.word() }),
            lock == crate::profile_edit::WorkspaceLock::Preparing,
        ),
        WorkspaceRefusal::Changing => coded(
            PortClass::Conflict,
            WORKSPACE_LOCKED,
            serde_json::json!({ "reason": WORKSPACE_CHANGING }),
            true,
        ),
        WorkspaceRefusal::NotFound => not_found(token),
        WorkspaceRefusal::Store(error) => refusal_error(COMMAND, PortRefusal::Store(error)),
    }
}

/// An LLM creation door (policy header ④): the original's backend must be one this surface creates.
// ADR-0279
// ADR-0386
fn verb_duplicate(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentDuplicateArgs,
) -> Result<AgentDuplicateOk, CommandError> {
    let token = args.target.as_str();
    reject_blanks(&[("target", Some(token), Blank::NeedsValue)])?;
    let id = resolve(host, token)?.id;
    let original = host.agent_snapshot(id).ok_or_else(|| not_found(token))?;
    let word = crate::launch::backend_word(&original.command);
    if let Some(reason) = llm_creation_refusal(&word) {
        return Err(policy_refusal(&word, reason));
    }
    let copy = host.duplicate_agent(id).map_err(|e| match e {
        PtyError::NotFound(_) => not_found(token),
        other => create_error(other, &original.cwd.to_string_lossy()),
    })?;
    notify.roster_changed();
    Ok(AgentDuplicateOk {
        agent_id: copy.id.to_string(),
        name: copy.canonical_name_when_live(),
        state: AGENT_STATE_SLEEPING.to_string(),
    })
}

// ★The two verbs do not change the roster — they emit no roster notification★ (the waiting list flows through the agent output stream).
// ADR-0231
fn verb_list_queued_inputs(
    host: &dyn AgentCommandHost,
    args: AgentListQueuedInputsArgs,
) -> Result<AgentListQueuedInputsOk, CommandError> {
    let token = args.target.as_str();
    reject_blanks(&[("target", Some(token), Blank::NeedsValue)])?;
    let id = resolve(host, token)?.id;
    let listing = host
        .list_queued_inputs(id)
        .ok_or_else(|| not_running(token))?;
    Ok(AgentListQueuedInputsOk {
        inputs: listing
            .rows
            .into_iter()
            .map(|row| QueuedInputRow {
                id: row.id,
                text: row.text,
                state: row.state.as_str().to_string(),
                cancel: match row.state {
                    ListedState::Cancelling {
                        answer,
                        vendor_closed,
                    } => Some(QueuedInputCancel {
                        answer: answer.as_str().to_string(),
                        vendor_closed,
                    }),
                    ListedState::Queued | ListedState::Sent | ListedState::Unconfirmed => None,
                },
            })
            .collect(),
        as_of_seq: listing.as_of_seq,
        epoch: listing.epoch,
        stopped_after_error: listing.stopped_after_error,
    })
}

// ADR-0231
fn verb_cancel_queued_input(
    host: &dyn AgentCommandHost,
    args: AgentCancelQueuedInputArgs,
) -> Result<AgentCancelQueuedInputOk, CommandError> {
    let (token, input_id) = (args.target.as_str(), args.input_id.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("input_id", Some(input_id), Blank::NeedsValue),
    ])?;
    // ★The text carries the roster name, not the pointing token★ — the common entry rewrites `target` as the resolved id
    //   (daemon `admit_input`), so the token arriving here is usually a UUID.
    let agent = resolve(host, token)?;
    let name = agent.name.as_str();
    match host.cancel_queued_input(agent.id, input_id) {
        Ok(outcome) => Ok(AgentCancelQueuedInputOk {
            outcome: outcome.as_str().to_string(),
        }),
        // ★「already has an outcome」 and 「asleep」 are not told apart★ — the caller has the same thing to do (look at the list again).
        Err(CancelError::NotFound) => Err(CommandError::not_found(format!(
            "'{name}' has no waiting input '{input_id}' — it was already delivered, cancelled or dropped, or the agent is not running; list its queued inputs to see what is still waiting"
        ))),
        // ★It does not guide to call again★: the entry stays pending cancel, so calling again writes nothing and returns `requested`
        //   (idempotent). The outcome is decided by the agent's own lifecycle.
        Err(CancelError::Write(e)) => Err(CommandError::internal(format!(
            "the cancel request for '{input_id}' could not be written to '{name}': {e} — the item stays in the cancelling state and calling again will not resend it; its fate now follows the agent's own lifecycle, so watch it in the queued-input list"
        ))),
    }
}

// ADR-0237
fn verb_interrupt(
    host: &dyn AgentCommandHost,
    args: AgentInterruptArgs,
) -> Result<AgentInterruptOk, CommandError> {
    let token = args.target.as_str();
    reject_blanks(&[("target", Some(token), Blank::NeedsValue)])?;
    // The text carries the roster name — the common entry rewrites `target` as the resolved id (same as `verb_cancel_queued_input`).
    let agent = resolve(host, token)?;
    let name = agent.name.as_str();
    match host.interrupt_agent(agent.id) {
        // ★`requested` is not 「stopped」★ — it only sent the interrupt, and the stop is announced by the turn-end event.
        Ok(()) => Ok(AgentInterruptOk {
            outcome: "requested".to_string(),
        }),
        Err(PtyError::NotFound(_)) => Err(CommandError::not_found(format!(
            "'{name}' is not running, so it has no turn to interrupt"
        ))),
        // The two reasons (no turn to interrupt · a link that cannot interrupt) are not told apart — the link answers with one variant and
        //   the caller
        //   has the same thing to do.
        Err(PtyError::Unsupported(reason)) => Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "'{name}' has no turn to interrupt, or this agent does not support interrupting: {reason}"
            ),
        )),
        Err(other) => Err(CommandError::internal(format!(
            "could not interrupt '{name}': {other}"
        ))),
    }
}

/// A change with a caller that the store (profiles) did not accept → a caller error (code + retry instruction). The bus and WS (daemon `connection_core`) use this, and the shell settings (`settings.set` · `settings.reset`) follow the same split:
/// - A store refused at load (read-only for this whole run, any cause) = `CONFLICT` · `never` — clears only by fixing or replacing the file and restarting the daemon.
/// - A refusal by the re-decision right before writing — file written by a newer edition = `CONFLICT` · `never` (same while that file stays) · unreadable = `CONFLICT` · `after-condition` (usually a briefly held lock).
/// - Write failure = `INTERNAL`.
///
/// ★In all three the roster is unchanged★ — a change with a caller commits only if the save succeeds (the `try_` versions of registry · manager), so `CONFLICT` means what the daemon reads as 「a rejection with nothing applied」 (`command_delivery::retains_the_id`).
/// ★Today the retry instruction does not reach the caller★ — the socket bus exit lowers it to `never` (daemon `command_delivery::send_reply` · ADR-0159) and the control route's answer carries no such field, so 「again a little later」 is carried by the text
/// ([`store_error_reason`]) and the help.
// ADR-0291 R17
pub fn store_command_error(error: &StoreError, message: String) -> CommandError {
    match error {
        StoreError::Refused(Refusal::Unreadable) => {
            CommandError::with_retry(ErrorCode::Conflict, message, RetryMode::AfterCondition)
        }
        StoreError::ReadOnly(_) | StoreError::Refused(Refusal::Newer { .. }) => {
            CommandError::of(ErrorCode::Conflict, message)
        }
        StoreError::Io(_) => CommandError::internal(message),
    }
}

/// The reason carried in the caller text — which file (`file`) did not accept and why, and what the caller should do. ★English on purpose: the LLM reads it★ (the stores' own `Display` is for logs).
// ADR-0291 R17
pub fn store_error_reason(file: &str, error: &StoreError) -> String {
    let cause = store_error_cause(file, error);
    match error {
        StoreError::ReadOnly(Refusal::Newer { .. }) => format!(
            "{cause}; retrying will not help (use the newer build or replace the file, then restart the daemon)"
        ),
        StoreError::ReadOnly(Refusal::Unreadable) => format!(
            "{cause}; retrying will not help (fix or replace the file, then restart the daemon)"
        ),
        StoreError::Refused(Refusal::Newer { .. }) => {
            format!("{cause}; retrying will not help while that file is there")
        }
        StoreError::Refused(Refusal::Unreadable) => {
            format!("{cause}; this is usually a brief lock, so retry in a moment")
        }
        StoreError::Io(_) => cause,
    }
}

/// The same reason, cause only — which file (`file`) did not accept and why. For requests where an instruction does not fit (a launch failure after creating — the text's tail gives what to do).
// ADR-0291 R17
fn store_error_cause(file: &str, error: &StoreError) -> String {
    match error {
        StoreError::ReadOnly(Refusal::Newer { found }) => format!(
            "{file} was written by a newer build (version {found}) and is read-only for this daemon run"
        ),
        StoreError::ReadOnly(Refusal::Unreadable) => format!(
            "{file} could not be read when the daemon started and is read-only for this run"
        ),
        StoreError::Refused(Refusal::Newer { found }) => format!(
            "{file} was written by a newer build (version {found}) and was not overwritten"
        ),
        StoreError::Refused(Refusal::Unreadable) => {
            format!("{file} could not be read just now and was not overwritten")
        }
        StoreError::Io(io) => format!("writing {file} failed: {io}"),
    }
}

/// Returns the caller error (`failure`) of a change the store did not accept as is, while recording it — ★the one line recording the refusal is here alone★ (where that request becomes an error on this surface). The store's log is the file-side fact (what blocked),
/// this line the request-side fact (which request). `action` = what it was trying to do. Caller-typed values (names · paths) are not logged.
// ADR-0291 R17
pub(crate) fn store_error(
    action: &'static str,
    error: &StoreError,
    failure: CommandError,
) -> CommandError {
    tracing::warn!(
        action,
        file = PROFILES_FILE,
        %error,
        "the store did not accept it, so this request is returned as an error (command bus)"
    );
    failure
}

/// It is in the roster but there is no live session — only a live incarnation holds a waiting list.
fn not_running(token: &str) -> CommandError {
    CommandError::not_found(format!(
        "'{token}' is not running, so it holds no queued inputs — an asleep agent has nothing waiting"
    ))
}

fn current_name(host: &dyn AgentCommandHost, id: AgentId) -> Option<String> {
    host.roster()
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.canonical_name)
}

fn started_payload(started: StartedAgent, created: bool) -> AgentSpawnOk {
    AgentSpawnOk {
        agent_id: started.id.to_string(),
        name: started.name,
        state: state_of(Some(&started.status)).to_string(),
        created,
        notices: started.notices.into_iter().map(launch_notice_row).collect(),
    }
}

/// A launch notice as the declared row — its text by [`notice_text`].
pub fn launch_notice_row(notice: Notice) -> LaunchNoticeRow {
    LaunchNoticeRow {
        text: notice_text(&notice),
        code: notice.code,
        args: notice.args,
        line: notice.line,
    }
}

/// The start of every `agent.spawn` refusal of a launch before any process (a block or a race — `CONFLICT`). Such a
/// message ends with its own next step, so a surface adds no recovery tail of its own (daemon `control::agent`).
pub const LAUNCH_REFUSAL: &str = "launch refused";

/// Why a launch raced — the cause only; the next step belongs to the message that carries it.
const LAUNCH_RACED_CAUSE: &str = "its launch inputs (presets or their library files, command or env) changed while it was being prepared";

/// Why a launch lost its reservation to a working-folder change — the cause only, as [`LAUNCH_RACED_CAUSE`].
// ADR-0386
const WORKING_FOLDER_CHANGING_CAUSE: &str =
    "its working folder is being changed (agent.setWorkspace)";

/// The cause of a launch block, without a next step — English, like the rest of this surface. The reasons
/// themselves travel in the error's args ([`LaunchBlocked::to_args`]), so `LaunchBlocked`'s `Display` is not
/// used here.
fn launch_blocked_reason(blocked: &LaunchBlocked) -> String {
    format!(
        "its launch is blocked — {} reason(s) in args.reasons (a broken or unreadable item, a conflict, or a library mount that cannot be read)",
        blocked.reasons.len()
    )
}

/// **The single mapping** of state → wire vocabulary; the predicate is [`AgentStatus::is_live`] alone (ADR-0116).
///
/// ★No literal baked in (it diverged here)★: a change verb baking `"live"` would report an agent that dies right after waking (ADR-0082 resume early exit) as alive by `spawn` and asleep by the next `list`, and the caller would write mail to a corpse.
fn state_of(status: Option<&AgentStatus>) -> &'static str {
    match status {
        Some(s) if s.is_live() => AGENT_STATE_LIVE,
        _ => AGENT_STATE_SLEEPING,
    }
}

/// Pointing token → one agent.
///
/// ★Rules (must be **the same** as the mail entry)★: ① an agent id match first — a *name* that looks like a UUID cannot hijack id pointing; ② then **exact** name match (case-sensitive · no prefix · no whitespace correction); ③ the same name two or more times → nobody chosen, refused.
/// ★Do not bring in a lenient resolver★: people and LLMs read the mail recipient token and the control pointing token as the same X; diverging resolution would make the agent that got the mail differ from the one renamed — guarded by the daemon crate's cross-check
/// test running [`resolve_in`].
// ADR-0132
pub(crate) fn resolve(
    host: &dyn AgentCommandHost,
    token: &str,
) -> Result<ResolvedAgent, CommandError> {
    resolve_in(&host.roster(), token)
}

/// The pure kernel of [`resolve`] — **the very resolution rule the control entry uses**.
///
/// ★Two reasons it is `pub`★: ① the daemon crate's cross-check test must run the real resolver to measure it is **the same rule** as the mail entry; taking the roster as an argument avoids imitating the whole `AgentCommandHost` and keeps no separate copy.
/// ② the daemon's input lease check (`control::commands::admit_input`) resolves the `target` of [`INPUT_AFFECTING`] commands **by the same rule** as the verb body; two resolvers would let the agent checked and the agent executed diverge.
/// ★The outcome is read by code★: absent = `NOT_FOUND` · same name two or more = `CONFLICT`. Both texts end with ` — candidates: a, b` (≤5 · sorted) — absent = names worth choosing (`none` if the roster is empty), same name = those ids. The text is prose, not a
/// contract (CLI · cross-check look only at the code).
// ADR-0132
// ADR-0155
// ADR-0231
// ADR-0307
pub fn resolve_in(roster: &[AgentRosterRow], token: &str) -> Result<ResolvedAgent, CommandError> {
    let found = |row: &AgentRosterRow| ResolvedAgent {
        id: row.id,
        name: row.canonical_name.clone(),
    };
    // ★Not compared in canonical spelling★: the daemon converts shell commands' agent fields (including the old `agent_id`) with this
    //   function (ADR-0308), so canonical-only matching would give old callers sending ids in other spellings (e.g. uppercase) `NOT_FOUND`.
    //   An id-shaped value not in the roster falls through to the name.
    if let Ok(id) = AgentId::parse_str(token) {
        if let Some(row) = roster.iter().find(|r| r.id == id) {
            return Ok(found(row));
        }
    }
    let same_name: Vec<&AgentRosterRow> = roster
        .iter()
        .filter(|r| r.canonical_name == token)
        .collect();
    match same_name.as_slice() {
        [row] => Ok(found(row)),
        [] => Err(CommandError::not_found(with_candidates(
            not_found_message(token),
            name_candidates(roster, token),
        ))),
        rows => {
            let mut ids: Vec<String> = rows.iter().map(|row| row.id.to_string()).collect();
            ids.sort_unstable();
            ids.truncate(CANDIDATES_MAX);
            Err(CommandError::of(
                ErrorCode::Conflict,
                with_candidates(
                    format!(
                        "more than one agent is called '{token}', so this command would have to guess — use the agent id instead; the roster listing shows both"
                    ),
                    ids,
                ),
            ))
        }
    }
}

// The cap of TRD S28 §5-3.
const CANDIDATES_MAX: usize = 5;

/// Name candidates for a missing word — names containing it (case-insensitive) if any, otherwise the first ones of all in name order; the same name once.
// ★Sorted★: the roster's row order is not fixed (asleep rows come from a new HashMap each time — `AgentManager::roster`); cutting in that
//   order would give the same request different candidates each call.
fn name_candidates(roster: &[AgentRosterRow], token: &str) -> Vec<String> {
    let mut names: Vec<&str> = roster
        .iter()
        .map(|row| row.canonical_name.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    let folded = token.to_lowercase();
    let containing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| name.to_lowercase().contains(&folded))
        .collect();
    let picked = if containing.is_empty() {
        names
    } else {
        containing
    };
    picked
        .into_iter()
        .take(CANDIDATES_MAX)
        .map(str::to_string)
        .collect()
}

/// `none` if there are no candidates (an empty roster) — dropping the tail would make 「no candidates」 indistinguishable from 「a text that carries no candidates」.
fn with_candidates(message: String, candidates: Vec<String>) -> String {
    if candidates.is_empty() {
        return format!("{message} — candidates: none");
    }
    format!("{message} — candidates: {}", candidates.join(", "))
}

/// The agent the pointing pointed at — the id and **the name the roster said at that time**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    pub id: AgentId,
    pub name: String,
}

fn not_found(token: &str) -> CommandError {
    CommandError::not_found(not_found_message(token))
}

fn not_found_message(token: &str) -> String {
    format!(
        "no agent called '{token}' — names are matched exactly, with no case-folding, prefixes or trimming; the roster listing shows the names in use"
    )
}

/// A whitespace-only value is **rejected rather than folded into absence**.
///
/// ★An unset shell variable expanding to an empty argument (`--parent "$UNSET"`) realistically comes in★ — folding it as "not given" would turn one typo into a different action (hierarchy release etc.).
/// ★Emitted **all gathered**, not cut per field★: cutting at the first would make a caller sending three blanks repeat rejection and fix three times (like the entry's unknown-field check — `CommandTable::check_args`).
/// ★Guidance differs **per field** (load-bearing)★: a caller (especially an LLM) rejected again after doing as told circles the same place — telling a required field "may be omitted" gets the omitting retry rejected as 「missing」, and telling a field where `null` is an
/// active instruction only "fill in a value" hides that branch. Each field gets **the way that works for that field** ([`Blank`]).
pub(crate) fn reject_blanks(fields: &[(&str, Option<&str>, Blank)]) -> Result<(), CommandError> {
    let advice: Vec<String> = fields
        .iter()
        .filter(|(_, given, _)| is_blank(*given))
        .map(|(field, _, what)| match what {
            Blank::NeedsValue => format!("{field} needs a real value"),
            Blank::OrOmit => format!("{field} must either carry a value or be left out entirely"),
            Blank::OrElse(alternative) => format!("{field} needs {alternative}"),
        })
        .collect();
    if advice.is_empty() {
        return Ok(());
    }
    Err(CommandError::invalid_argument(format!(
        "blank value(s) — an empty argument is usually an unset shell variable: {}",
        advice.join("; ")
    )))
}

/// **What actually works** for a field that came blank.
pub(crate) enum Blank<'a> {
    /// A value is required (omitting it is rejected as 「missing」).
    NeedsValue,
    /// Putting a value in or omitting the field entirely works.
    OrOmit,
    /// There is a branch other than a value — that branch is carried in the text as is (e.g. a field where `null` is an active instruction).
    OrElse(&'a str),
}

fn is_blank(given: Option<&str>) -> bool {
    given.is_some_and(|v| v.trim().is_empty())
}

/// All names of fields that came blank — used when **the verb writes** the rejection text directly (places where the constraint is a combination, not one field).
fn blank_fields<'a>(fields: &[(&'a str, Option<&str>)]) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|(_, given)| is_blank(*given))
        .map(|(field, _)| *field)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use engram_dashboard_command::testing::block_on;
    use serde_json::json;

    use super::*;
    use crate::launch::{notice_row, BlockReason, Reason};

    /// Fake manager — ★neither a process nor a PTY★. If this harness does not stand, T-1 is broken.
    #[derive(Default)]
    struct FakeHost {
        rows: Mutex<Vec<AgentRosterRow>>,
        profiles: Mutex<HashMap<AgentId, AgentProfile>>,
        started: Mutex<Vec<(AgentId, bool)>>,
        rename_result: Mutex<Option<RenameOutcome>>,
        reparent_ok: Mutex<bool>,
        start_fails: Mutex<bool>,
        /// The next activation's error (taken once) — a launch block or race.
        start_error: Mutex<Option<PtyError>>,
        /// The notices every successful activation returns.
        start_notices: Mutex<Vec<Notice>>,
        /// The next create's error (taken once) — `create_agent`'s, and for `create_and_activate` a refusal before
        /// the add (nothing registered).
        create_fails: Mutex<Option<PtyError>>,
        /// Every `create_and_activate` call — the profile the body built and the delivery it asked for.
        creates: Mutex<Vec<(AgentProfile, NoticeDelivery)>>,
        /// When on, the three that change the roster and activation come back with a save refusal (a file written by a newer edition) — the roster is unchanged.
        // ADR-0291 R17
        store_refuses: Mutex<bool>,
        /// The store state `agent.list` carries — writable by default.
        // ADR-0291 R17 (D6)
        store_status: Mutex<StoreStatus>,
        /// The target to delete from the roster **inside** the reparent call — reproduces 「vanished in the meantime」 (child or parent). ★Deleted regardless of apply success★: after a failure it exercises the reason branch, after success the response name's fallback;
        /// one side only would leave the other side's code unreached.
        vanish_on_reparent: Mutex<Option<AgentId>>,
        /// The state `activate_profile` returns. ★It must be possible to put a terminal one in★ — dying right after launching (ADR-0082 resume early exit) is the only way to measure whether the response state is made up; fixed at `Running`, baking `"live"` into
        ///   `started_payload` would keep the suite green.
        started_status: Mutex<Option<AgentStatus>>,
        /// The new name the child gets right after a reparent succeeds — reproduces 「a rename slipped in after applying」.
        rename_on_reparent: Mutex<Option<String>>,
        /// The answer of the waiting-list query. `None` = no live session.
        queued_listing: Mutex<Option<QueuedListing>>,
        /// The answer of the queued input cancellation (used once and emptied). `NotFound` if empty.
        cancel_answer: Mutex<Option<Result<CancelOutcome, CancelError>>>,
        /// The pointing the two verbs handed to the manager — measures whether the resolved id goes as is.
        queued_calls: Mutex<Vec<(AgentId, Option<String>)>>,
        /// The answer of the interrupt (used once and emptied). `Ok(())` if empty.
        interrupt_answer: Mutex<Option<Result<(), PtyError>>>,
        /// The pointing the interrupt handed to the manager.
        interrupt_calls: Mutex<Vec<AgentId>>,
        /// The next `set_workspace` answer (taken once). Empty = set: the agent's row name and the folder as given.
        workspace_answer: Mutex<Option<Result<WorkspaceSet, WorkspaceRefusal>>>,
        /// What the handler handed `set_workspace` — the resolved id and the folder as typed.
        workspace_calls: Mutex<Vec<(AgentId, String)>>,
        /// The next `duplicate_agent` answer (taken once). Empty = a copy named `<name>(1)`, not registered.
        duplicate_answer: Mutex<Option<Result<AgentProfile, PtyError>>>,
        /// What the handler handed `duplicate_agent`.
        duplicate_calls: Mutex<Vec<AgentId>>,
    }

    /// Roster notification counter — the watcher of 「renamed but the tree shows the old roster」.
    #[derive(Default)]
    struct FakeNotify {
        calls: Mutex<usize>,
    }

    impl RosterChanged for FakeNotify {
        fn roster_changed(&self) {
            *self.calls.lock().expect("notify poisoned") += 1;
        }
    }

    /// Fake usage port — records the received backend and returns the planted row (`None` = no field).
    #[derive(Default)]
    struct FakeUsage {
        row: Mutex<Option<UsageVendorRow>>,
        calls: Mutex<Vec<(&'static str, AgentBackend)>>,
    }

    impl UsageCommandHost for FakeUsage {
        fn get(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
            self.calls.lock().unwrap().push(("get", backend.clone()));
            self.row.lock().unwrap().clone()
        }

        fn refresh(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
            self.calls
                .lock()
                .unwrap()
                .push(("refresh", backend.clone()));
            self.row.lock().unwrap().clone()
        }
    }

    fn wiring(host: &Arc<FakeHost>) -> (CommandTable, Arc<FakeNotify>) {
        wiring_with_library(
            host,
            crate::library_commands::fake::FakeLibrary::unplugged(),
        )
    }

    fn wiring_with_library(
        host: &Arc<FakeHost>,
        library: Arc<dyn LibraryCommandHost>,
    ) -> (CommandTable, Arc<FakeNotify>) {
        let notify = Arc::new(FakeNotify::default());
        let table = make_table(
            Arc::clone(host) as Arc<dyn AgentCommandHost>,
            Arc::clone(&notify) as Arc<dyn RosterChanged>,
            Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
            library,
        );
        (table, notify)
    }

    impl FakeHost {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                reparent_ok: Mutex::new(true),
                ..Self::default()
            })
        }

        /// If save refusal is on, it returns before touching the roster, like the manager's `try_` versions.
        fn refuse_if_set(&self) -> Result<(), PtyError> {
            if *self.store_refuses.lock().unwrap() {
                return Err(PtyError::Store(StoreError::ReadOnly(
                    crate::profile::Refusal::Newer { found: 2 },
                )));
            }
            Ok(())
        }

        fn with_agent(self: &Arc<Self>, name: &str, live: bool, resumable: bool) -> AgentId {
            self.with_status(name, live.then_some(AgentStatus::Running), resumable)
        }

        /// Plants the row's state **as is** — to measure what word a terminal-state row comes out as in `list`,
        /// something other than `Running` must be insertable too.
        fn with_status(
            self: &Arc<Self>,
            name: &str,
            live: Option<AgentStatus>,
            resumable: bool,
        ) -> AgentId {
            self.with_status_command(
                name,
                live,
                resumable,
                AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: NEW_AGENT_OUTPUT_FORMAT,
                },
            )
        }

        /// The variant deciding even the row's **command** — to measure whether activation mode derivation looks at the backend axis, commands other than claude
        /// must be plantable too (planting only claude makes that decision always give the same answer, splitting nothing).
        fn with_status_command(
            self: &Arc<Self>,
            name: &str,
            live: Option<AgentStatus>,
            resumable: bool,
            command: AgentCommand,
        ) -> AgentId {
            let profile = AgentProfile::new(
                format!("C:/work/{name}"),
                command,
                PathBuf::from(format!("C:/work/{name}")),
                vec![],
                false,
            );
            let id = profile.id;
            let mut profile = profile;
            profile.display_name = Some(name.to_string());
            if resumable {
                profile.backend_session_id = Some(uuid::Uuid::new_v4());
            }
            self.profiles.lock().unwrap().insert(id, profile);
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live,
            });
            id
        }
    }

    /// A fake's conditional create: the fakes hold no registry lock, so `still` is asked right before the create —
    /// enough for a handler test on one thread.
    fn ask_then_create(
        host: &dyn AgentCommandHost,
        profile: AgentProfile,
        still: &dyn Fn() -> bool,
    ) -> Result<Option<AgentProfile>, PtyError> {
        if !still() {
            return Ok(None);
        }
        host.create_agent(profile).map(Some)
    }

    impl AgentCommandHost for FakeHost {
        fn roster(&self) -> Vec<AgentRosterRow> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .map(|r| AgentRosterRow {
                    id: r.id,
                    canonical_name: r.canonical_name.clone(),
                    cwd: r.cwd.clone(),
                    parent: r.parent,
                    live: r.live.clone(),
                })
                .collect()
        }

        fn store_status(&self) -> StoreStatus {
            *self.store_status.lock().unwrap()
        }

        fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
            self.profiles.lock().unwrap().get(&id).cloned()
        }

        fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
            if let Some(failure) = self.create_fails.lock().unwrap().take() {
                return Err(failure);
            }
            self.refuse_if_set()?;
            let id = profile.id;
            let name = profile.canonical_name_when_live();
            self.profiles.lock().unwrap().insert(id, profile.clone());
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name,
                cwd: profile.cwd.to_string_lossy().to_string(),
                parent: None,
                live: None,
            });
            Ok(profile)
        }

        fn create_agent_if(
            &self,
            profile: AgentProfile,
            still: &dyn Fn() -> bool,
        ) -> Result<Option<AgentProfile>, PtyError> {
            ask_then_create(self, profile, still)
        }

        fn activate_profile(
            &self,
            profile: &AgentProfile,
            mode: SpawnMode,
        ) -> Result<StartedAgent, PtyError> {
            if *self.start_fails.lock().unwrap() {
                return Err(PtyError::SpawnFailed("fake".to_string()));
            }
            if let Some(error) = self.start_error.lock().unwrap().take() {
                return Err(error);
            }
            self.refuse_if_set()?;
            let resumed = matches!(mode, SpawnMode::Resume);
            self.started.lock().unwrap().push((profile.id, resumed));
            let status = self
                .started_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(AgentStatus::Running);
            Ok(StartedAgent {
                id: profile.id,
                name: profile.canonical_name_when_live(),
                status,
                notices: self.start_notices.lock().unwrap().clone(),
            })
        }

        /// The real split: a refusal before the add registers nothing; after the add every start failure leaves the
        /// row (`created: true`).
        fn create_and_activate(
            &self,
            profile: &AgentProfile,
            delivery: NoticeDelivery,
        ) -> Result<Activated, NotStarted> {
            self.creates
                .lock()
                .unwrap()
                .push((profile.clone(), delivery));
            self.create_agent(profile.clone())
                .map_err(|error| NotStarted {
                    error,
                    created: false,
                })?;
            let started = self
                .activate_profile(profile, SpawnMode::Fresh)
                .map_err(|error| NotStarted {
                    error,
                    created: true,
                })?;
            Ok(Activated {
                info: fake_info(&started),
                notices: started.notices,
            })
        }

        fn rename_agent(
            &self,
            _id: AgentId,
            display_name: Option<String>,
        ) -> Result<RenameOutcome, PtyError> {
            self.refuse_if_set()?;
            Ok(self
                .rename_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| RenameOutcome::Renamed(display_name.unwrap_or_default())))
        }

        fn reparent_agent(
            &self,
            child: AgentId,
            _parent: Option<AgentId>,
        ) -> Result<bool, PtyError> {
            self.refuse_if_set()?;
            let ok = *self.reparent_ok.lock().unwrap();
            // ★It vanishes regardless of apply success★: the window vanishing **after** success is the only path that exercises the
            //   response name's fallback.
            //   Putting it only on the failure side leaves that fallback unreached by any test.
            if let Some(vanished) = *self.vanish_on_reparent.lock().unwrap() {
                self.rows.lock().unwrap().retain(|row| row.id != vanished);
            }
            // The window where a rename is committed between applying and responding — in reality another caller issues it.
            if let (true, Some(renamed)) = (ok, self.rename_on_reparent.lock().unwrap().take()) {
                for row in self.rows.lock().unwrap().iter_mut() {
                    if row.id == child {
                        row.canonical_name = renamed.clone();
                    }
                }
            }
            Ok(ok)
        }

        fn set_workspace(&self, id: AgentId, cwd: &str) -> Result<WorkspaceSet, WorkspaceRefusal> {
            self.workspace_calls
                .lock()
                .unwrap()
                .push((id, cwd.to_string()));
            self.workspace_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| {
                    let name = self
                        .rows
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|row| row.id == id)
                        .map(|row| row.canonical_name.clone())
                        .unwrap_or_default();
                    Ok(WorkspaceSet {
                        name,
                        cwd: cwd.to_string(),
                    })
                })
        }

        fn duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError> {
            self.duplicate_calls.lock().unwrap().push(id);
            self.duplicate_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| {
                    let original = self.agent_snapshot(id).ok_or(PtyError::NotFound(id))?;
                    let mut copy = original.clone();
                    copy.id = uuid::Uuid::new_v4();
                    copy.display_name = Some(format!("{}(1)", original.canonical_name_when_live()));
                    Ok(copy)
                })
        }

        fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
            self.queued_calls.lock().unwrap().push((id, None));
            self.queued_listing.lock().unwrap().clone()
        }

        fn cancel_queued_input(
            &self,
            id: AgentId,
            input_id: &str,
        ) -> Result<CancelOutcome, CancelError> {
            self.queued_calls
                .lock()
                .unwrap()
                .push((id, Some(input_id.to_string())));
            self.cancel_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Err(CancelError::NotFound))
        }

        fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
            self.interrupt_calls.lock().unwrap().push(id);
            self.interrupt_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(()))
        }
    }

    fn call(
        table: &CommandTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        // Called through the table — the only place fitting the arguments to the declared schema (`CommandTable::call`).
        let future = table.call(name, &mut args).expect("a name in the table");
        block_on(future)
    }

    /// The `AgentInfo` of a fake start — only the fields this table reads come from `started`.
    fn fake_info(started: &StartedAgent) -> crate::types::AgentInfo {
        use crate::types::{
            BackendCaps, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
            TransportCaps,
        };
        crate::types::AgentInfo {
            id: started.id,
            name: started.name.clone(),
            cwd: String::new(),
            status: started.status.clone(),
            cols: 80,
            rows: 24,
            epoch: 1,
            capabilities: Capabilities::compose(
                TransportCaps {
                    input: InputCaps {
                        raw: true,
                        message: false,
                        attachment: false,
                    },
                    output: OutputCaps {
                        terminal_bytes: true,
                        structured: false,
                        markdown: false,
                        tool_events: false,
                        usage: false,
                    },
                    control: ControlCaps {
                        resize: true,
                        interrupt: false,
                        cancel: false,
                        graceful_shutdown: false,
                    },
                },
                BackendCaps {
                    session: SessionCaps {
                        resume: false,
                        snapshot: false,
                        cwd_env: true,
                    },
                    model: ModelCaps {
                        select: false,
                        temperature: false,
                        max_tokens: false,
                    },
                },
            ),
            reads_messages: false,
        }
    }

    #[test]
    fn table_holds_exactly_the_declared_verbs() {
        let (table, _notify) = wiring(&FakeHost::new());
        let names: Vec<&str> = table.specs().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "agent.attachPreset",
                "agent.cancelQueuedInput",
                "agent.detachPreset",
                "agent.duplicate",
                "agent.interrupt",
                "agent.list",
                "agent.listQueuedInputs",
                "agent.move",
                "agent.new",
                "agent.presets",
                "agent.rename",
                "agent.reorderPresets",
                "agent.resolve",
                "agent.setWorkspace",
                "agent.spawn",
                "library.addMount",
                "library.copy",
                "library.delete",
                "library.emptyTrash",
                "library.folders",
                "library.list",
                "library.listTrash",
                "library.move",
                "library.newFile",
                "library.newFolder",
                "library.read",
                "library.refresh",
                "library.relink",
                "library.relocateMount",
                "library.removeMount",
                "library.rename",
                "library.renameMount",
                "library.restore",
                "library.status",
                "preset.add",
                "preset.check",
                "preset.get",
                "preset.previewFolder",
                "preset.remove",
                "preset.reorder",
                "preset.save",
                "preset.setBackend",
                "usage.get",
                "usage.refresh"
            ]
        );
        assert_eq!(names.len(), COMMAND_SPECS.len());
    }

    /// ADR-0369 · ADR-0419: `agent.spawn` launches and the six library file writes can outlast the ordinary wait (a
    /// copy across volumes, emptying a large trash), so a shell-socket seat takes the launch bound and the CLI waits by
    /// the launch budget; every other verb here — `library.listTrash` among them — keeps the shared waits.
    #[test]
    fn only_agent_spawn_and_the_library_file_writes_are_declared_with_the_launch_deadline() {
        let mut launches: Vec<&str> = COMMAND_SPECS
            .iter()
            .filter(|spec| spec.deadline == engram_dashboard_command::DeadlineClass::Launch)
            .map(|spec| spec.name)
            .collect();
        launches.sort_unstable();
        assert_eq!(
            launches,
            [
                "agent.spawn",
                "library.copy",
                "library.delete",
                "library.emptyTrash",
                "library.move",
                "library.rename",
                "library.restore"
            ]
        );
    }

    #[test]
    fn decls_carry_a_non_empty_json_shape_for_every_verb() {
        let (table, _notify) = wiring(&FakeHost::new());
        for decl in table.decls() {
            let item: serde_json::Value =
                serde_json::from_str(&decl.help).expect("help is one JSON item");
            assert_eq!(item["name"], decl.name.as_str());
            assert!(item["args"]["type"] == "object");
            assert!(item["ok"]["type"] == "object");
            assert!(item["errors"].as_array().is_some_and(|e| !e.is_empty()));
        }
    }

    #[test]
    fn list_reports_live_and_sleeping_without_a_process() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        host.with_agent("beta", false, false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        let agents = out["agents"].as_array().expect("array");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["name"], "alpha");
        assert_eq!(agents[0]["state"], "live");
        assert_eq!(agents[1]["state"], "sleeping");
        assert_eq!(agents[0]["parent"], serde_json::Value::Null);
    }

    /// ★The three store states are carried in `agent.list` with their reasons (D6)★ — without `Refusing`, a refusal during the run would look `Writable` while changes come back `CONFLICT`, leaving the error unexplained.
    // ADR-0291 R17 (D6)
    #[test]
    fn list_reports_the_store_status_with_its_reason() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        assert_eq!(
            out["store"],
            json!({ "agents": { "state": "Writable", "reason": null, "file_version": null } })
        );

        for (status, expected) in [
            (
                StoreStatus::ReadOnly(Refusal::Newer { found: 3 }),
                json!({ "state": "ReadOnly", "reason": "Newer", "file_version": 3 }),
            ),
            (
                StoreStatus::Refusing(Refusal::Unreadable),
                json!({ "state": "Refusing", "reason": "Unreadable", "file_version": null }),
            ),
            (
                StoreStatus::Refusing(Refusal::Newer { found: 4 }),
                json!({ "state": "Refusing", "reason": "Newer", "file_version": 4 }),
            ),
            (
                StoreStatus::ReadOnly(Refusal::Unreadable),
                json!({ "state": "ReadOnly", "reason": "Unreadable", "file_version": null }),
            ),
        ] {
            *host.store_status.lock().unwrap() = status;
            let out = call(&table, "agent.list", json!({})).expect("query succeeds");
            assert_eq!(out["store"], json!({ "agents": expected }), "{status:?}");
        }
    }

    /// ★The real manager carries the roster store's state (D6)★ — the fake only returns planted fields, so a manager always leaving that state `Writable` is caught only with the real thing.
    // ADR-0291 R17 (D6)
    #[test]
    fn a_real_manager_reports_its_store_status() {
        use crate::profile::{ProfileRegistry, ProfileStore};
        use crate::session_tracker::{SessionTracker, TrackerConfig};
        use crate::types::{AgentInfo, StatusSink};

        struct NoStatus;
        impl StatusSink for NoStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }
        struct NewerAgents;
        impl ProfileStore for NewerAgents {
            fn save(&self, _profiles: &[AgentProfile]) -> Result<(), StoreError> {
                Err(StoreError::ReadOnly(Refusal::Newer { found: 9 }))
            }
            fn load(&self) -> Vec<AgentProfile> {
                vec![]
            }
            fn status(&self) -> StoreStatus {
                StoreStatus::ReadOnly(Refusal::Newer { found: 9 })
            }
        }

        let manager = Arc::new(AgentManager::new(
            Arc::new(NoStatus),
            Arc::new(ProfileRegistry::new(Arc::new(NewerAgents))),
            Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: std::time::Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            )),
        ));
        let table = make_table(
            manager as Arc<dyn AgentCommandHost>,
            Arc::new(FakeNotify::default()) as Arc<dyn RosterChanged>,
            Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
            crate::library_commands::fake::FakeLibrary::unplugged(),
        );

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        assert_eq!(
            out["store"],
            json!({
                "agents": { "state": "ReadOnly", "reason": "Newer", "file_version": 9 },
            })
        );
    }

    #[test]
    fn spawn_refuses_both_target_and_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "cwd": "C:/x" }),
        )
        .expect_err("giving both is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_neither_target_nor_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.spawn", json!({})).expect_err("giving nothing is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_name_when_waking() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("name can do nothing for waking");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn waking_a_stored_session_resumes_it() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, true);
        let (table, notify) = wiring(&host);

        let out = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
        assert_eq!(out["created"], false);
        assert_eq!(out["state"], "live");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, true)]);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "even if the composition is unchanged, activation changes the last failure (ADR-0172) — that axis flows only through the profile list"
        );
    }

    /// ★A failure from an LLM wake-up must reach the screen★: without this notification a human click (WS `SpawnProfile`) shows the failure marker but the command bus does not — the two handles would not move the same thing.
    // ADR-0172
    #[test]
    fn waking_that_fails_still_announces_the_roster() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, true);
        *host.start_fails.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("premise: activation fails");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ★Resume is not derived from a stored sid alone (ADR-0185)★ — a sid on a command that cannot resume exists today (a hand-edited profile file · a profile that switched backends); launching it as Resume **opens a new conversation but reports 「resumed」**, and the
    /// wire has no marker for a new conversation, so neither screen nor LLM can tell. ★The WS entry has the same pair★ (CLAUDE.md 「LLM-first control」).
    // ADR-0185
    /// ★The sample being shell is intended — do not bring the codex terminal mode back here★: by ADR-0208 that mode uses `codex resume <id>` and its resume axis is **true**; shell is the only sample with the axis off, and replacing it would remove what this entry measures.
    #[test]
    fn waking_does_not_resume_a_profile_whose_backend_cannot_resume() {
        let host = FakeHost::new();
        let id = host.with_status_command(
            "shell-with-stale-sid",
            None,
            true,
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
        );
        let (table, _notify) = wiring(&host);

        call(
            &table,
            "agent.spawn",
            json!({ "target": "shell-with-stale-sid" }),
        )
        .expect("wake");
        assert_eq!(
            host.started.lock().unwrap().as_slice(),
            &[(id, false)],
            "even with a sid, a command that cannot resume must launch Fresh"
        );
    }

    /// ★The pair of the entry above — the codex terminal mode now stands **on the opposite side** (ADR-0208)★: with the handle in the roster it must also launch as Resume; without this the entry above would stay green even if the axis were turned off entirely.
    // ADR-0208
    #[test]
    fn waking_a_codex_terminal_profile_with_a_stored_handle_resumes() {
        let host = FakeHost::new();
        let id = host.with_status_command(
            "codex-term",
            None,
            true,
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: CoreAgentOutputFormat::Terminal,
            },
        );
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "codex-term" })).expect("wake");
        assert_eq!(
            host.started.lock().unwrap().as_slice(),
            &[(id, true)],
            "if it launched Fresh while a handle the hook recorded exists, that id would never be used"
        );
    }

    #[test]
    fn waking_without_a_stored_session_starts_fresh() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, false)]);
    }

    /// The bus create goes through the one body: one `create_and_activate` (the Fresh activation path) that delivers
    /// the notices, with the normalized folder, the given name and the shared default launch command — and one roster
    /// notify, after the activation.
    // ADR-0348
    #[test]
    fn spawn_by_cwd_creates_then_starts() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.spawn",
            json!({ "cwd": r#"C:\work\gamma"#, "name": "gamma" }),
        )
        .expect("create + launch");
        assert_eq!(out["created"], true);
        assert_eq!(out["name"], "gamma");
        assert_eq!(host.started.lock().unwrap().len(), 1);
        let creates = host.creates.lock().unwrap();
        assert_eq!(creates.len(), 1, "one create-and-activate call");
        let (profile, delivery) = &creates[0];
        assert_eq!(*delivery, NoticeDelivery::Delivered);
        assert_eq!(profile.cwd, PathBuf::from("C:/work/gamma"));
        assert_eq!(profile.display_name.as_deref(), Some("gamma"));
        assert_eq!(
            profile.command,
            backend_command(AgentBackend::Claude, NEW_AGENT_OUTPUT_FORMAT)
        );
        assert_eq!(out["agent_id"], profile.id.to_string());
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "one notify after the activation (its last-failure outcome, ADR-0172) — none before it"
        );
    }

    #[test]
    fn a_created_agent_that_fails_to_start_is_not_rolled_back() {
        let host = FakeHost::new();
        *host.start_fails.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "cwd": "C:/work/gamma" }))
            .expect_err("start fails");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(
            host.rows.lock().unwrap().len(),
            1,
            "it remains in the roster"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "the asleep row is announced"
        );
    }

    // ── Launch answers (TRD S25 step 16) ──

    fn blocked() -> LaunchBlocked {
        LaunchBlocked {
            reasons: vec![
                BlockReason::Missing {
                    item: "rules/a.md".into(),
                    via: vec!["team.yaml".into()],
                    reason: Reason {
                        code: "not-found".into(),
                        args: json!({ "path": "rules/a.md" }),
                        line: Some(3),
                    },
                },
                BlockReason::LibraryUnavailable {
                    reason: Reason {
                        code: "library-unset".into(),
                        args: json!({}),
                        line: None,
                    },
                },
            ],
        }
    }

    /// A wake refused before any process is a `CONFLICT` — the daemon then releases the request id
    /// (`command_delivery::retains_the_id`), so the retry after the fix is not swallowed — and carries every reason in
    /// `args`. The roster is still announced (the failure is recorded — ADR-0172).
    #[test]
    fn a_blocked_wake_is_a_conflict_carrying_the_reasons() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.start_error.lock().unwrap() = Some(PtyError::LaunchBlocked(blocked()));
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("the launch is blocked");

        let message = err.message();
        assert_eq!(err.code(), ErrorCode::Conflict, "{message}");
        assert_eq!(err.retry(), RetryMode::Never, "{message}");
        assert_eq!(err.args(), Some(&blocked().to_args()), "{message}");
        assert!(
            message.starts_with(&format!("{LAUNCH_REFUSAL} for agent 'alpha'"))
                && message.contains("2 reason(s)")
                && message.ends_with("fix the listed items, then start it again"),
            "{message}"
        );
        assert!(host.started.lock().unwrap().is_empty(), "nothing started");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    #[test]
    fn a_raced_wake_is_a_conflict_to_try_again() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.start_error.lock().unwrap() = Some(PtyError::LaunchRaced);
        let (table, _notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("the launch raced");

        let message = err.message();
        assert_eq!(err.code(), ErrorCode::Conflict, "{message}");
        assert_eq!(err.retry(), RetryMode::AfterCondition, "{message}");
        assert_eq!(err.args(), None, "{message}");
        assert!(
            message.starts_with(&format!("{LAUNCH_REFUSAL} for agent 'alpha'"))
                && message.ends_with("try again"),
            "{message}"
        );
    }

    /// A wake that lost the reservation to a working-folder change is a retryable `CONFLICT` launch refusal — not
    /// `INTERNAL`, which would keep the request id and swallow the retry (ADR-0386).
    #[test]
    fn a_wake_during_a_working_folder_change_is_a_conflict_to_retry_after_it() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.start_error.lock().unwrap() = Some(PtyError::WorkingFolderChanging);
        let (table, _notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("the working folder is being changed");

        let message = err.message();
        assert_eq!(err.code(), ErrorCode::Conflict, "{message}");
        assert_eq!(err.retry(), RetryMode::AfterCondition, "{message}");
        assert_eq!(err.args(), None, "{message}");
        assert!(
            message.starts_with(&format!("{LAUNCH_REFUSAL} for agent 'alpha'"))
                && message.ends_with("start it again once that change is done"),
            "{message}"
        );
    }

    // ── agent.setWorkspace · agent.duplicate (ADR-0386) ──

    #[test]
    fn set_workspace_hands_the_resolved_id_and_the_typed_folder_and_announces() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.setWorkspace",
            json!({ "target": "alpha", "cwd": "C:/new" }),
        )
        .expect("set");

        assert_eq!(
            out,
            json!({ "agent_id": id.to_string(), "name": "alpha", "cwd": "C:/new" })
        );
        assert_eq!(
            *host.workspace_calls.lock().unwrap(),
            vec![(id, "C:/new".to_string())]
        );
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    #[test]
    fn set_workspace_refusals_carry_their_code_and_args_and_announce_nothing() {
        let coded =
            |code: &str, args: serde_json::Value| Some(json!({ "code": code, "args": args }));
        let cases = vec![
            (
                WorkspaceRefusal::FolderNotFound {
                    cwd: "C:/gone".into(),
                    why: "timeout".into(),
                },
                ErrorCode::InvalidArgument,
                coded(
                    WORKSPACE_NOT_FOUND,
                    json!({ "cwd": "C:/gone", "why": "timeout" }),
                ),
            ),
            (
                WorkspaceRefusal::Locked(crate::profile_edit::WorkspaceLock::Running),
                ErrorCode::Conflict,
                coded(WORKSPACE_LOCKED, json!({ "reason": "running" })),
            ),
            (
                WorkspaceRefusal::Locked(crate::profile_edit::WorkspaceLock::Preparing),
                ErrorCode::Conflict,
                coded(WORKSPACE_LOCKED, json!({ "reason": "preparing" })),
            ),
            (
                WorkspaceRefusal::Locked(crate::profile_edit::WorkspaceLock::Conversed),
                ErrorCode::Conflict,
                coded(WORKSPACE_LOCKED, json!({ "reason": "conversed" })),
            ),
            (
                WorkspaceRefusal::Changing,
                ErrorCode::Conflict,
                coded(WORKSPACE_LOCKED, json!({ "reason": "changing" })),
            ),
            (WorkspaceRefusal::NotFound, ErrorCode::NotFound, None),
            (
                WorkspaceRefusal::Store(StoreError::Io(std::io::Error::other("disk full"))),
                ErrorCode::Internal,
                None,
            ),
            (
                WorkspaceRefusal::Store(StoreError::ReadOnly(crate::profile::Refusal::Newer {
                    found: 2,
                })),
                ErrorCode::Conflict,
                None,
            ),
        ];
        for (refusal, code, args) in cases {
            let label = format!("{refusal:?}");
            let host = FakeHost::new();
            host.with_agent("alpha", false, false);
            *host.workspace_answer.lock().unwrap() = Some(Err(refusal));
            let (table, notify) = wiring(&host);

            let err = call(
                &table,
                "agent.setWorkspace",
                json!({ "target": "alpha", "cwd": "C:/gone" }),
            )
            .expect_err(&label);

            assert_eq!(err.code(), code, "{label}: {}", err.message());
            assert_eq!(
                err.args().cloned().map(serde_json::Value::Object),
                args,
                "{label}"
            );
            assert!(
                err.message().starts_with("agent.setWorkspace: ") || code == ErrorCode::NotFound,
                "{label}: {}",
                err.message()
            );
            assert_eq!(*notify.calls.lock().unwrap(), 0, "{label}");
        }
    }

    /// The refusals that clear by themselves — a stat that timed out, a share not reachable, a launch preparing,
    /// another change — carry the retry directive and its words; the ones only a different call or state fixes do not
    /// (ADR-0386 decision 14 · ADR-0387).
    #[test]
    fn set_workspace_refusals_that_clear_by_themselves_say_retry() {
        use crate::profile_edit::WorkspaceLock;
        let folder = |why: &str| WorkspaceRefusal::FolderNotFound {
            cwd: "C:/w".into(),
            why: why.into(),
        };
        let cases = vec![
            (folder(crate::launch::STAT_TIMED_OUT), true),
            (folder(crate::launch::SHARE_NOT_REACHABLE), true),
            (WorkspaceRefusal::Locked(WorkspaceLock::Preparing), true),
            (WorkspaceRefusal::Changing, true),
            (folder("missing"), false),
            (folder("not a folder"), false),
            (folder("not absolute"), false),
            (WorkspaceRefusal::Locked(WorkspaceLock::Running), false),
            (WorkspaceRefusal::Locked(WorkspaceLock::Conversed), false),
        ];
        for (refusal, clearable) in cases {
            let label = format!("{refusal:?}");
            let host = FakeHost::new();
            host.with_agent("alpha", false, false);
            *host.workspace_answer.lock().unwrap() = Some(Err(refusal));
            let (table, _notify) = wiring(&host);

            let err = call(
                &table,
                "agent.setWorkspace",
                json!({ "target": "alpha", "cwd": "C:/w" }),
            )
            .expect_err(&label);

            assert_eq!(
                err.retry() == RetryMode::AfterCondition,
                clearable,
                "{label}: {:?}",
                err.retry()
            );
            assert_eq!(
                err.message()
                    .ends_with("clear the cause, then repeat the same call"),
                clearable,
                "{label}: {}",
                err.message()
            );
        }
    }

    #[test]
    fn set_workspace_rejects_blanks_and_resolves_the_target_first() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);
        for args in [
            json!({ "target": "alpha", "cwd": "  " }),
            json!({ "target": " ", "cwd": "C:/new" }),
        ] {
            let err = call(&table, "agent.setWorkspace", args.clone()).expect_err("blank");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
        }
        let err = call(
            &table,
            "agent.setWorkspace",
            json!({ "target": "ghost", "cwd": "C:/new" }),
        )
        .expect_err("no such agent");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(host.workspace_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn duplicate_registers_through_the_host_and_announces() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        let out = call(&table, "agent.duplicate", json!({ "target": "alpha" })).expect("copied");

        assert_eq!(out["name"], "alpha(1)");
        assert_eq!(out["state"], AGENT_STATE_SLEEPING);
        assert_ne!(out["agent_id"], id.to_string());
        assert_eq!(*host.duplicate_calls.lock().unwrap(), vec![id]);
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// A duplicate is an LLM creation door: an original whose backend this surface does not create is refused with the
    /// policy text naming that backend, before the host is called (ADR-0279 · ADR-0386).
    #[test]
    fn duplicate_refuses_an_original_whose_backend_the_policy_closes() {
        let host = FakeHost::new();
        host.with_status_command(
            "sh",
            None,
            false,
            AgentCommand::Shell {
                program: "sh".into(),
                args: vec![],
            },
        );
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.duplicate", json!({ "target": "sh" })).expect_err("closed");

        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{}", err.message());
        assert!(
            err.message().starts_with("backend 'shell' is a known word"),
            "{}",
            err.message()
        );
        assert!(host.duplicate_calls.lock().unwrap().is_empty());
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    #[test]
    fn duplicate_refusals_map_like_a_create_and_announce_nothing() {
        let cases = [
            (
                PtyError::RosterFull {
                    current: 512,
                    limit: 512,
                },
                ErrorCode::Conflict,
            ),
            (
                PtyError::Unsupported("name suffix space exhausted".into()),
                ErrorCode::Conflict,
            ),
            (
                PtyError::NotFound(uuid::Uuid::new_v4()),
                ErrorCode::NotFound,
            ),
            (
                PtyError::Store(StoreError::Io(std::io::Error::other("disk full"))),
                ErrorCode::Internal,
            ),
        ];
        for (error, code) in cases {
            let label = error.to_string();
            let host = FakeHost::new();
            host.with_agent("alpha", false, false);
            *host.duplicate_answer.lock().unwrap() = Some(Err(error));
            let (table, notify) = wiring(&host);

            let err =
                call(&table, "agent.duplicate", json!({ "target": "alpha" })).expect_err(&label);

            assert_eq!(err.code(), code, "{label}: {}", err.message());
            assert_eq!(*notify.calls.lock().unwrap(), 0, "{label}");
        }
    }

    /// With `cwd` the agent exists before its start fails, so the answer stays `INTERNAL` — a `CONFLICT` would release
    /// the request id and the same id's retry would create a second agent (ADR-0122). Every such answer names the
    /// created agent in `args.agent_id` (a block adds its reasons) so the recovery starts that one. The text carries
    /// no instruction but its tail: no "try again" (it reads as "create again"), and a block says fix first — waiting
    /// does not clear it.
    #[test]
    fn a_start_failure_after_the_create_stays_internal_and_names_the_created_agent() {
        const AGAIN: &str = "start it again by that name (in a moment) instead of creating another";
        const FIX_FIRST: &str =
            "fix the listed items first, then start it by that name instead of creating another";
        let cases = [
            (
                PtyError::LaunchBlocked(blocked()),
                blocked().to_args(),
                FIX_FIRST,
            ),
            (PtyError::LaunchRaced, serde_json::Map::new(), AGAIN),
            (
                PtyError::SpawnFailed("fake".into()),
                serde_json::Map::new(),
                AGAIN,
            ),
            (
                PtyError::Store(StoreError::Io(std::io::Error::other("disk full"))),
                serde_json::Map::new(),
                AGAIN,
            ),
        ];
        for (error, mut want, tail) in cases {
            let host = FakeHost::new();
            *host.start_error.lock().unwrap() = Some(error);
            let (table, notify) = wiring(&host);

            let err = call(
                &table,
                "agent.spawn",
                json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
            )
            .expect_err("the launch is refused after the create");

            let message = err.message();
            assert_eq!(err.code(), ErrorCode::Internal, "{message}");
            let rows = host.rows.lock().unwrap();
            assert_eq!(rows.len(), 1, "the created agent stays");
            want.insert("agent_id".into(), rows[0].id.to_string().into());
            assert_eq!(err.args(), Some(&want), "{message}");
            assert!(
                message.contains("was created but did not start") && message.ends_with(tail),
                "{message}"
            );
            assert!(!message.contains("try again"), "{message}");
            assert_eq!(
                message.matches("fix the listed items").count(),
                usize::from(tail == FIX_FIRST),
                "{message}"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                1,
                "once, after the activation"
            );
        }
    }

    /// The success answer carries the activation's notices as rows of the [`notice_row`] shape (`line` is `null`
    /// when the notice names none), and an empty list when there are none.
    #[test]
    fn the_spawn_answer_carries_the_launch_notices_as_rows() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let quiet = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
        assert_eq!(quiet["notices"], json!([]), "{quiet}");

        let lined = Reason {
            code: "omitted".into(),
            args: json!({ "names": ["a.md"], "backend": "codex" }),
            line: Some(4),
        };
        let bare = Reason {
            code: "other".into(),
            args: json!({}),
            line: None,
        };
        *host.start_notices.lock().unwrap() = vec![lined.clone(), bare.clone()];
        let out =
            call(&table, "agent.spawn", json!({ "cwd": "C:/work/gamma" })).expect("create + start");

        let mut bare_row = notice_row(&bare);
        bare_row["line"] = serde_json::Value::Null;
        assert_eq!(
            out["notices"],
            json!([notice_row(&lined), bare_row]),
            "{out}"
        );
    }

    // ── The one creation body (TRD S25 step 20 — ADR-0348) ──

    fn open_policy(_word: &str) -> Option<&'static str> {
        None
    }

    fn new_agent(backend: AgentBackend, policy: fn(&str) -> Option<&'static str>) -> NewAgent {
        NewAgent {
            cwd: "C:/work/gamma".into(),
            name: Some("gamma".into()),
            backend,
            policy,
            delivery: NoticeDelivery::Delivered,
        }
    }

    /// A block found before the add creates nothing, so it is a `CONFLICT` — the daemon releases the request id and the
    /// fixed retry runs — carrying every reason, with no notify and no row.
    #[test]
    fn a_cwd_launch_blocked_before_the_add_is_a_conflict_that_creates_nothing() {
        let host = FakeHost::new();
        *host.create_fails.lock().unwrap() = Some(PtyError::LaunchBlocked(blocked()));
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "cwd": r#"C:\work\gamma"#, "name": "gamma" }),
        )
        .expect_err("the launch is blocked before the add");

        let message = err.message();
        assert_eq!(err.code(), ErrorCode::Conflict, "{message}");
        assert_eq!(err.retry(), RetryMode::Never, "{message}");
        assert_eq!(err.args(), Some(&blocked().to_args()), "{message}");
        assert!(
            message.starts_with(&format!(
                "{LAUNCH_REFUSAL} for a new agent in 'C:/work/gamma'"
            )) && message.contains("2 reason(s)")
                && message.contains("nothing was created")
                && message.ends_with("fix the listed items, then try again"),
            "{message}"
        );
        assert!(host.rows.lock().unwrap().is_empty(), "nothing registered");
        assert!(host.started.lock().unwrap().is_empty(), "nothing started");
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// *Defensive* — the real pre-create check returns only a block; the mapping stays total for a race.
    #[test]
    fn a_cwd_launch_raced_before_the_add_is_a_conflict_to_try_again() {
        let host = FakeHost::new();
        *host.create_fails.lock().unwrap() = Some(PtyError::LaunchRaced);
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "cwd": "C:/work/gamma" }))
            .expect_err("the launch raced before the add");

        let message = err.message();
        assert_eq!(err.code(), ErrorCode::Conflict, "{message}");
        assert_eq!(err.retry(), RetryMode::AfterCondition, "{message}");
        assert_eq!(err.args(), None, "{message}");
        assert!(
            message.starts_with(&format!(
                "{LAUNCH_REFUSAL} for a new agent in 'C:/work/gamma'"
            )) && message.contains("nothing was created")
                && message.ends_with("try again"),
            "{message}"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// The policy is asked before anything else: a closed backend never reaches the host and announces nothing. The
    /// bus answer is `agent.new`'s policy wording (`INVALID_ARGUMENT`).
    #[test]
    fn the_body_refuses_a_closed_backend_before_calling_the_host() {
        fn closes_claude(word: &str) -> Option<&'static str> {
            word.eq_ignore_ascii_case("claude")
                .then_some("closed for this test")
        }
        let host = FakeHost::new();
        let notify = FakeNotify::default();

        let refused = create_and_start(
            host.as_ref(),
            &notify,
            new_agent(AgentBackend::Claude, closes_claude),
        )
        .expect_err("the policy closes claude");

        assert!(
            matches!(
                refused,
                CreateRefused::Policy {
                    reason: "closed for this test"
                }
            ),
            "{refused:?}"
        );
        assert!(host.creates.lock().unwrap().is_empty(), "host not called");
        assert!(host.rows.lock().unwrap().is_empty());
        assert_eq!(*notify.calls.lock().unwrap(), 0);

        let err = spawn_create_refusal(refused, "C:/work/gamma", "Claude");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("backend 'Claude'")
                && err.message().ends_with("closed for this test"),
            "{}",
            err.message()
        );
        assert_eq!(err.args(), None);
    }

    /// The body asks with the declared spelling (`Claude`, `Codex`) — the same word `agent.new` asks with.
    #[test]
    fn the_body_asks_the_policy_with_the_declared_spelling() {
        static ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn recording(word: &str) -> Option<&'static str> {
            ASKED.lock().unwrap().push(word.to_string());
            None
        }
        for backend in [AgentBackend::Claude, AgentBackend::Codex] {
            let host = FakeHost::new();
            create_and_start(
                host.as_ref(),
                &FakeNotify::default(),
                new_agent(backend, recording),
            )
            .expect("an open policy creates");
        }
        assert_eq!(*ASKED.lock().unwrap(), vec!["Claude", "Codex"]);
    }

    /// One roster notify, after the activation, exactly when the agent was registered — a start (1), a start failure
    /// after the add (1, the asleep row and its last failure reach the screen only this way), a refusal before the add
    /// (0) or by the policy (0).
    #[test]
    fn the_body_announces_the_roster_once_exactly_when_it_registered_an_agent() {
        let started = FakeHost::new();
        let notify = FakeNotify::default();
        create_and_start(
            started.as_ref(),
            &notify,
            new_agent(AgentBackend::Claude, open_policy),
        )
        .expect("starts");
        assert_eq!(*notify.calls.lock().unwrap(), 1, "a start");

        let asleep = FakeHost::new();
        *asleep.start_fails.lock().unwrap() = true;
        let notify = FakeNotify::default();
        let refused = create_and_start(
            asleep.as_ref(),
            &notify,
            new_agent(AgentBackend::Claude, open_policy),
        )
        .expect_err("the start fails after the add");
        let rows = asleep.rows.lock().unwrap();
        match refused {
            CreateRefused::CreatedAsleep { id, name, .. } => {
                assert_eq!(id, rows[0].id, "the registered agent");
                assert_eq!(name, "gamma", "its registered name");
            }
            other => panic!("created asleep expected: {other:?}"),
        }
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "a start failure after the add"
        );

        let not_created = FakeHost::new();
        *not_created.create_fails.lock().unwrap() = Some(PtyError::RosterFull {
            current: 40,
            limit: 40,
        });
        let notify = FakeNotify::default();
        let refused = create_and_start(
            not_created.as_ref(),
            &notify,
            new_agent(AgentBackend::Claude, open_policy),
        )
        .expect_err("refused before the add");
        assert!(
            matches!(
                refused,
                CreateRefused::NotCreated(PtyError::RosterFull { .. })
            ),
            "{refused:?}"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 0, "a refusal before the add");
    }

    /// The body hands the caller's delivery to the host as given — a withheld caller must not mark the set as shown.
    #[test]
    fn the_body_passes_the_callers_delivery_to_the_host() {
        for delivery in [NoticeDelivery::Withheld, NoticeDelivery::Delivered] {
            let host = FakeHost::new();
            create_and_start(
                host.as_ref(),
                &FakeNotify::default(),
                NewAgent {
                    delivery,
                    ..new_agent(AgentBackend::Claude, open_policy)
                },
            )
            .expect("creates");
            let creates = host.creates.lock().unwrap();
            assert_eq!(creates.len(), 1);
            assert_eq!(creates[0].1, delivery);
        }
    }

    /// A created-asleep answer names the agent by the name it was registered under, which can differ from the
    /// requested one (roster uniqueness suffix) — the caller wakes it by that name.
    #[test]
    fn a_created_asleep_answer_names_the_registered_name_not_the_requested_one() {
        // Registers the agent under a suffixed name (as the real roster does when the name is taken), then fails to
        // start it.
        struct SuffixesThenFails(Arc<FakeHost>);
        impl AgentCommandHost for SuffixesThenFails {
            fn roster(&self) -> Vec<AgentRosterRow> {
                self.0.roster()
            }
            fn store_status(&self) -> StoreStatus {
                self.0.store_status()
            }
            fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
                self.0.agent_snapshot(id)
            }
            fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
                self.0.create_agent(profile)
            }
            fn create_agent_if(
                &self,
                profile: AgentProfile,
                still: &dyn Fn() -> bool,
            ) -> Result<Option<AgentProfile>, PtyError> {
                ask_then_create(self, profile, still)
            }
            fn activate_profile(
                &self,
                profile: &AgentProfile,
                mode: SpawnMode,
            ) -> Result<StartedAgent, PtyError> {
                self.0.activate_profile(profile, mode)
            }
            fn create_and_activate(
                &self,
                profile: &AgentProfile,
                _delivery: NoticeDelivery,
            ) -> Result<Activated, NotStarted> {
                let mut registered = profile.clone();
                registered.display_name = Some("gamma(2)".into());
                self.0
                    .create_agent(registered)
                    .map_err(|error| NotStarted {
                        error,
                        created: false,
                    })?;
                Err(NotStarted {
                    error: PtyError::SpawnFailed("fake".into()),
                    created: true,
                })
            }
            fn rename_agent(
                &self,
                id: AgentId,
                display_name: Option<String>,
            ) -> Result<RenameOutcome, PtyError> {
                self.0.rename_agent(id, display_name)
            }
            fn reparent_agent(
                &self,
                child: AgentId,
                parent: Option<AgentId>,
            ) -> Result<bool, PtyError> {
                self.0.reparent_agent(child, parent)
            }
            fn set_workspace(
                &self,
                id: AgentId,
                cwd: &str,
            ) -> Result<WorkspaceSet, WorkspaceRefusal> {
                self.0.set_workspace(id, cwd)
            }
            fn duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError> {
                self.0.duplicate_agent(id)
            }
            fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
                self.0.list_queued_inputs(id)
            }
            fn cancel_queued_input(
                &self,
                id: AgentId,
                input_id: &str,
            ) -> Result<CancelOutcome, CancelError> {
                self.0.cancel_queued_input(id, input_id)
            }
            fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
                self.0.interrupt_agent(id)
            }
        }

        let host = FakeHost::new();
        let refused = create_and_start(
            &SuffixesThenFails(Arc::clone(&host)),
            &FakeNotify::default(),
            new_agent(AgentBackend::Claude, open_policy),
        )
        .expect_err("the start fails after the add");
        let id = host.rows.lock().unwrap()[0].id;
        match &refused {
            CreateRefused::CreatedAsleep { id: got, name, .. } => {
                assert_eq!(*got, id);
                assert_eq!(
                    name, "gamma(2)",
                    "the registered name, not the requested 'gamma'"
                );
            }
            other => panic!("created asleep expected: {other:?}"),
        }
        let err = spawn_create_refusal(refused, "C:/work/gamma", "Claude");
        assert!(
            err.message().starts_with(&format!(
                "agent 'gamma(2)' ({id}) was created but did not start"
            )),
            "{}",
            err.message()
        );
    }

    /// Door ③ of the policy table: the bus cwd arm hands the production policy to the body. A behaviour test cannot
    /// tell it from an always-open policy today (the table closes nothing), so the wiring is pinned in the source.
    // ADR-0348
    #[test]
    fn the_bus_cwd_arm_hands_the_production_policy_to_the_body() {
        let src = include_str!("commands.rs");
        let production = src.split("mod tests {").next().expect("production part");
        let arm: String = production
            .split("fn verb_spawn(")
            .nth(1)
            .expect("`verb_spawn` body")
            .split("fn wake_existing(")
            .next()
            .expect("up to the next function")
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .collect::<String>()
            .split_whitespace()
            .collect();
        assert!(
            arm.contains("(None,Some(cwd))=>create_and_start(")
                && arm.contains("policy:llm_creation_refusal,"),
            "the bus cwd arm does not hand `llm_creation_refusal` to the body: {arm}"
        );
    }

    /// The body fills one shared default mode for every backend — the entrances carry no mode field, so the body
    /// invents the value; [`NEW_AGENT_OUTPUT_FORMAT`] is its only home (its literal is pinned by
    /// `the_new_agent_default_output_format_is_pinned_to_a_literal`).
    #[test]
    fn the_body_builds_one_shared_default_launch_command_for_every_backend() {
        for backend in [AgentBackend::Claude, AgentBackend::Codex] {
            let host = FakeHost::new();
            create_and_start(
                host.as_ref(),
                &FakeNotify::default(),
                new_agent(backend.clone(), open_policy),
            )
            .expect("creates");
            let creates = host.creates.lock().unwrap();
            let command = &creates[0].0.command;
            match (&backend, command) {
                (
                    AgentBackend::Claude,
                    AgentCommand::Claude {
                        extra_args,
                        output_format,
                    },
                )
                | (
                    AgentBackend::Codex,
                    AgentCommand::Codex {
                        extra_args,
                        output_format,
                    },
                ) => {
                    assert!(extra_args.is_empty(), "{command:?}");
                    assert_eq!(*output_format, NEW_AGENT_OUTPUT_FORMAT, "{command:?}");
                }
                _ => panic!("{backend:?} built {command:?}"),
            }
        }
    }

    /// The created-asleep text names the agent, carries the cause without an instruction, and ends with the one next
    /// step — fix first for a block (waiting does not clear it), start it again by that name otherwise.
    #[test]
    fn created_but_not_started_names_the_agent_and_says_what_to_do_per_error_kind() {
        let id = AgentId::new_v4();
        let head = format!("agent 'gamma' ({id}) was created but did not start: ");
        const AGAIN: &str = " — it is registered and asleep, so start it again by that name (in a moment) instead of creating another";
        let cases = [
            (
                PtyError::LaunchBlocked(blocked()),
                format!(
                    "{} — it is registered and asleep, so fix the listed items first, then start it by that name instead of creating another",
                    launch_blocked_reason(&blocked())
                ),
            ),
            (
                PtyError::LaunchRaced,
                format!("{LAUNCH_RACED_CAUSE}{AGAIN}"),
            ),
            (
                PtyError::Store(StoreError::Refused(crate::profile::Refusal::Unreadable)),
                format!("{PROFILES_FILE} could not be read just now and was not overwritten{AGAIN}"),
            ),
            (
                PtyError::SpawnFailed("os error 267".into()),
                format!("spawn failed: os error 267{AGAIN}"),
            ),
        ];
        for (error, tail) in cases {
            assert_eq!(
                created_but_not_started("gamma", id, &error),
                format!("{head}{tail}"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn new_registers_a_sleeping_agent() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
        )
        .expect("register");
        assert_eq!(out["state"], "sleeping");
        assert!(host.started.lock().unwrap().is_empty(), "does not launch");
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "announces the roster change"
        );
    }

    // ── `agent.new {presets}` (ADR-0386 decision 5) ────────────────────────────────────────────────────

    /// With `presets`, the library judges them over the profile about to be created (the Default slot on, nothing
    /// attached) with the given root generation, before anything is registered; the agent is created holding exactly
    /// the attachments the judgement accepted, and the answer carries its skips as judged.
    #[test]
    fn new_with_presets_creates_the_agent_with_the_judged_attachments_and_answers_the_skips() {
        let host = FakeHost::new();
        let skipped = json!([{ "item": "M.yaml", "code": "not-found", "args": { "path": "M.yaml" }, "line": null }]);
        let library = crate::library_commands::fake::FakeLibrary::answering(Ok(json!({
            "default_preset": true, "presets": ["team/P.yaml", "Q.yaml"], "skipped": skipped,
        })));
        let (table, notify) =
            wiring_with_library(&host, Arc::clone(&library) as Arc<dyn LibraryCommandHost>);

        let out = call(
            &table,
            "agent.new",
            json!({
                "cwd": "C:/work/delta", "backend": "Claude",
                "presets": ["team/P.yaml", "M.yaml", "Q.yaml"], "root_generation": 41,
            }),
        )
        .expect("register");

        assert_eq!(
            library.calls(),
            vec![(
                "judge_new_presets",
                json!({
                    "profile": { "default_preset": true, "presets": [] },
                    "presets": ["team/P.yaml", "M.yaml", "Q.yaml"], "root_generation": 41,
                })
            )]
        );
        let id = AgentId::parse_str(out["agent_id"].as_str().expect("an id")).expect("a uuid");
        let created = host.profiles.lock().unwrap()[&id].clone();
        let attached: Vec<&str> = created.presets.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            (created.default_preset, attached),
            (true, vec!["team/P.yaml", "Q.yaml"]),
            "created with the accepted list"
        );
        assert_eq!(out["skipped"], skipped);
        assert_eq!(out["state"], "sleeping");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ADR-0400 decision 18: the insert compares the library's change number with the one the judgement was made at;
    /// a library change in between inserts nothing and the request — its paths as given — is judged again, and the
    /// agent is created with the second judgement's list and answers its skips.
    #[test]
    fn new_with_presets_judges_again_when_the_library_changed_before_the_insert() {
        let host = FakeHost::new();
        let library = crate::library_commands::fake::FakeLibrary::answering(Ok(json!({
            "default_preset": true, "presets": ["team/P.yaml"], "skipped": [], "change": 5,
        })));
        let skipped = json!([{ "item": "team/P.yaml", "code": "not-found", "args": { "path": "team/P.yaml" }, "line": null }]);
        library.then.lock().unwrap().push_back(Ok(json!({
            "default_preset": true, "presets": [], "skipped": skipped, "change": 6,
        })));
        library.change_numbers.lock().unwrap().extend([6, 6]);
        let (table, notify) =
            wiring_with_library(&host, Arc::clone(&library) as Arc<dyn LibraryCommandHost>);

        let out = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "Claude", "presets": ["team/P.yaml"] }),
        )
        .expect("register");

        let methods: Vec<&str> = library.calls().iter().map(|(method, _)| *method).collect();
        assert_eq!(
            methods,
            [
                "judge_new_presets",
                "change_number",
                "judge_new_presets",
                "change_number"
            ],
            "judged, compared (moved), judged again, compared (equal)"
        );
        let calls = library.calls();
        assert_eq!(
            calls[0].1, calls[2].1,
            "judged again with the paths as given"
        );
        let id = AgentId::parse_str(out["agent_id"].as_str().expect("an id")).expect("a uuid");
        let created = host.profiles.lock().unwrap()[&id].clone();
        assert!(created.presets.is_empty(), "the second judgement's list");
        assert_eq!(out["skipped"], skipped);
        assert_eq!(host.profiles.lock().unwrap().len(), 1, "created once");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// A judgement that refuses — a library preset with no library, a stale root generation — refuses the whole call
    /// with the library's code and creates nothing: no roster row, no notification.
    #[test]
    fn new_whose_presets_the_library_refuses_creates_nothing() {
        let cases = [
            (
                "library-unavailable",
                json!({ "message": "no root" }),
                RetryMode::AfterCondition,
            ),
            (
                "root-generation-stale",
                json!({ "given": 3, "current": 4 }),
                ErrorCode::Conflict.default_retry(),
            ),
        ];
        for (code, args, retry) in cases {
            let host = FakeHost::new();
            let library = crate::library_commands::fake::FakeLibrary::answering(Err(
                crate::library_commands::PortRefusal::Refused {
                    class: if code == "library-unavailable" {
                        crate::library_commands::PortClass::Unavailable
                    } else {
                        crate::library_commands::PortClass::Conflict
                    },
                    reason: Reason {
                        code: code.to_owned(),
                        args: args.clone(),
                        line: None,
                    },
                },
            ));
            let (table, notify) = wiring_with_library(&host, library);

            let err = call(
                &table,
                "agent.new",
                json!({
                    "cwd": "C:/work/delta", "backend": "Claude", "presets": ["P.yaml"],
                    "root_generation": 3,
                }),
            )
            .expect_err("refused");

            assert_eq!(
                (err.code(), err.retry()),
                (ErrorCode::Conflict, retry),
                "{code}"
            );
            assert_eq!(
                serde_json::Value::Object(err.args().expect("args").clone()),
                json!({ "code": code, "args": args }),
                "{code}"
            );
            assert!(
                host.rows.lock().unwrap().is_empty(),
                "{code}: nothing created"
            );
            assert!(host.profiles.lock().unwrap().is_empty(), "{code}");
            assert_eq!(*notify.calls.lock().unwrap(), 0, "{code}");
        }
    }

    /// Without `presets` nothing is judged — the library is never asked, so `root_generation` is not read even when
    /// given — and the agent is created as before, with the Default slot only and no skips.
    #[test]
    fn new_without_presets_asks_the_library_nothing() {
        let host = FakeHost::new();
        // Every call to this port refuses: a judgement would fail the create.
        let library = Arc::new(crate::library_commands::fake::FakeLibrary::default());
        let (table, _notify) =
            wiring_with_library(&host, Arc::clone(&library) as Arc<dyn LibraryCommandHost>);

        let out = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "Claude", "root_generation": 99 }),
        )
        .expect("register");

        assert!(library.calls().is_empty(), "the library was asked");
        assert_eq!(out["skipped"], json!([]));
        let id = AgentId::parse_str(out["agent_id"].as_str().expect("an id")).expect("a uuid");
        let created = host.profiles.lock().unwrap()[&id].clone();
        assert!(created.default_preset && created.presets.is_empty());
    }

    // ── Spelling of hand-typed words ───────────────────────────────────────────────────────────────

    /// Backend words are case-insensitive — this helper calling through the table actually runs the place where that adjustment happens
    /// (`CommandTable::call`).
    #[test]
    fn new_accepts_the_backend_word_in_any_casing() {
        for word in ["claude", "Claude", "CLAUDE", "codex", "Codex", "CODEX"] {
            let host = FakeHost::new();
            let (table, _notify) = wiring(&host);

            let out = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": word }),
            )
            .unwrap_or_else(|e| panic!("backend '{word}' must be accepted: {e:?}"));
            assert_eq!(out["state"], "sleeping", "{word}");
        }
    }

    /// Leniency applies only to **spelling** — an unknown word is rejected as is.
    #[test]
    fn new_still_refuses_a_backend_word_it_does_not_know() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "codx" }),
        )
        .expect_err("an unknown word is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    /// The emitted spelling does not change — the generated output (the `"enum"` of `bindings/commands.schema.json`) carries this value.
    #[test]
    fn the_declared_backend_words_still_serialize_in_pascal_case() {
        assert_eq!(
            serde_json::to_value(AgentBackend::Claude).expect("serialize"),
            json!("Claude")
        );
        assert_eq!(
            serde_json::to_value(AgentBackend::Codex).expect("serialize"),
            json!("Codex")
        );
    }

    /// Both create doors (`agent.new` · `agent.spawn --cwd`) put the same normalized spelling in the roster.
    #[test]
    fn a_windows_path_in_quotes_is_registered_with_the_normalized_spelling() {
        for verb in ["agent.new", "agent.spawn"] {
            let host = FakeHost::new();
            let (table, _notify) = wiring(&host);

            call(
                &table,
                verb,
                json!({ "cwd": r#""C:\work\delta""#, "backend": "Claude" }),
            )
            .unwrap_or_else(|e| panic!("{verb}: {e:?}"));

            let rows = host.rows.lock().unwrap();
            assert_eq!(rows.len(), 1, "{verb}");
            assert_eq!(rows[0].cwd, "C:/work/delta", "{verb}");
        }
    }

    /// Registration failures go out with a different code and retry instruction per reason — because what the caller should do differs.
    #[test]
    fn registration_failures_map_to_distinct_codes() {
        use crate::profile::Refusal;
        let cases = [
            (
                PtyError::RosterFull {
                    current: 40,
                    limit: 40,
                },
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::CwdDenied,
                ErrorCode::InvalidArgument,
                RetryMode::Never,
            ),
            (
                PtyError::SpawnFailed("disk on fire".to_string()),
                ErrorCode::Internal,
                RetryMode::Never,
            ),
            // ADR-0291 R17 — a refusal is `CONFLICT`; the retry instruction depends on origin: load refusal = `never` (clears only by
            //   restarting the daemon) · re-decision newer edition = `never` · re-decision unreadable = `after-condition` (usually a
            //   briefly held lock). Write failure = `INTERNAL`.
            (
                PtyError::Store(StoreError::ReadOnly(Refusal::Newer { found: 2 })),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::ReadOnly(Refusal::Unreadable)),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::Refused(Refusal::Newer { found: 2 })),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::Refused(Refusal::Unreadable)),
                ErrorCode::Conflict,
                RetryMode::AfterCondition,
            ),
            (
                PtyError::Store(StoreError::Io(std::io::Error::other("disk full"))),
                ErrorCode::Internal,
                RetryMode::Never,
            ),
        ];

        for (failure, expected, retry) in cases {
            let label = format!("{failure:?}");
            let from_store = matches!(failure, PtyError::Store(_));
            let host = FakeHost::new();
            *host.create_fails.lock().unwrap() = Some(failure);
            let (table, notify) = wiring(&host);

            let err = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
            )
            .expect_err("registration fails");
            assert_eq!(err.code(), expected, "{label}");
            assert_eq!(err.retry(), retry, "{label}");
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "if it failed, the roster did not change"
            );
            if from_store {
                // The text names which file did not accept and why — so the caller finds the place to fix.
                assert!(
                    err.message().contains(PROFILES_FILE)
                        && err.message().contains("nothing was registered"),
                    "{}",
                    err.message()
                );
            }
        }
    }

    /// ★A store refusal's reason is English and names what to do★ — the LLM reads it. A load refusal says restart; a re-decision's unreadable says again a little later (the retry instruction is lowered at the daemon exit, so the text carries it).
    // ADR-0291 R17
    #[test]
    fn store_refusal_reasons_are_english_and_name_the_way_out() {
        use crate::profile::Refusal;
        let at_load = store_error_reason("x.json", &StoreError::ReadOnly(Refusal::Unreadable));
        assert!(
            at_load.starts_with("x.json could not be read")
                && at_load.contains("restart the daemon"),
            "{at_load}"
        );
        let newer = store_error_reason("x.json", &StoreError::Refused(Refusal::Newer { found: 3 }));
        assert!(
            newer.starts_with("x.json was written by a newer build (version 3)")
                && newer.contains("retrying will not help"),
            "{newer}"
        );
        let transient = store_error_reason("x.json", &StoreError::Refused(Refusal::Unreadable));
        assert!(transient.contains("retry in a moment"), "{transient}");
        assert_eq!(
            store_error_reason(
                "x.json",
                &StoreError::Io(std::io::Error::other("disk full"))
            ),
            "writing x.json failed: disk full"
        );
    }

    /// ★No create without a working folder★ — an agent with no place to work sitting in the roster has no verb to remove it
    /// (ADR-0122).
    #[test]
    fn new_without_a_cwd_is_refused_and_registers_nothing() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.new", json!({ "backend": "Claude" }))
            .expect_err("no place to create");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(err.message().contains("cwd"), "{}", err.message());
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★`preset` is not a field of `agent.new`★ — the entry check (`CommandTable::check_args`) rejects it as an unknown field.
    ///   Deserialization drops unknown fields (the additive rule), so without this check an old call carrying a path bookmark id
    ///   would lose that field and be created with `cwd`.
    #[test]
    fn preset_is_not_an_argument_of_new() {
        let (table, _notify) = wiring(&FakeHost::new());

        let err = table
            .check_args(
                "agent.new",
                &json!({ "cwd": "C:/work/x", "preset": "bookmark-1", "backend": "Claude" }),
            )
            .expect_err("a field not in the declaration");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(err.message().contains(r#""preset""#), "{}", err.message());
    }

    /// ★Unspecified is StreamJson, values outside the vocabulary are rejected★ — with a silent fallback it would become "created it, but the screen is different"
    /// (ADR-0078) and the caller would never learn of their own typo.
    #[test]
    fn new_defaults_the_output_format_and_refuses_a_word_outside_the_vocabulary() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let format_of = |name: &str| {
            let id = host
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.canonical_name == name)
                .expect("registered row")
                .id;
            match host.profiles.lock().unwrap()[&id].command.clone() {
                AgentCommand::Claude { output_format, .. } => output_format,
                other => panic!("must be a claude profile: {other:?}"),
            }
        };

        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "silent", "backend": "Claude" }),
        )
        .expect("unspecified");
        assert_eq!(format_of("silent"), NEW_AGENT_OUTPUT_FORMAT);

        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "typed", "output_format": "Terminal", "backend": "Claude" }),
        )
        .expect("explicit");
        assert_eq!(format_of("typed"), CoreAgentOutputFormat::Terminal);

        // A spelling differing only in case is not outside the vocabulary — it is moved to the declared spelling and works.
        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "folded", "output_format": "terminal", "backend": "Claude" }),
        )
        .expect("a value differing only in spelling");
        assert_eq!(format_of("folded"), CoreAgentOutputFormat::Terminal);

        let err = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "output_format": "Ndjson", "backend": "Claude" }),
        )
        .expect_err("a value outside the vocabulary");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("Terminal") && err.message().contains("StreamJson"),
            "must say what works: {}",
            err.message()
        );
    }

    /// ★That a rejection **touches nothing in the roster** is decision material for another crate★: the daemon uses 「failure answer = nothing remained」 to decide whether to block re-queries after the deadline (`command_delivery::retains_the_id`); if it breaks, that side
    /// is **silently** wrong (holding a failed number blocks a normal retry; letting go of an applied number applies twice).
    /// ★`agent.spawn` is **absent** here — only that verb differs★: if it cannot launch after creating it answers `INTERNAL` with the agent left in the roster (`a_created_agent_that_fails_to_start_is_not_rolled_back` pins that), so the daemon splits that one code off as an exception.
    #[test]
    fn a_rejection_from_the_mutating_verbs_leaves_the_roster_untouched() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("lead", false, false);
        let (table, notify) = wiring(&host);
        let snapshot = || -> Vec<(AgentId, String, Option<AgentId>)> {
            host.roster()
                .into_iter()
                .map(|row| (row.id, row.canonical_name, row.parent))
                .collect()
        };
        let before = snapshot();

        // Sweeps the branches where rejections happen, per verb — the codes differ but all must be 「before touching anything」.
        let rejections = [
            ("agent.new", json!({ "cwd": "   ", "backend": "Claude" })),
            ("agent.new", json!({ "backend": "Claude" })),
            ("agent.rename", json!({ "target": "ghost", "name": "beta" })),
            ("agent.rename", json!({ "target": "alpha", "name": "  " })),
            ("agent.move", json!({ "target": "ghost", "parent": null })),
            ("agent.move", json!({ "target": "alpha" })),
        ];
        for (name, args) in rejections {
            let err =
                call(&table, name, args.clone()).expect_err("must be a rejection: {name} {args}");
            assert_ne!(
                err.code(),
                ErrorCode::Internal,
                "{name}: if this branch is INTERNAL, the daemon reads it as may-have-been-applied — {args}"
            );
            assert_eq!(
                snapshot(),
                before,
                "{name}: the rejection changed the roster — {args}"
            );
        }
        // ★The same holds for changes the store did not accept★ — the manager's `try_` versions commit only if the save succeeds, so the
        //   daemon's premise that `CONFLICT` is 「before touching anything」 stands.
        // ADR-0291 R17
        *host.store_refuses.lock().unwrap() = true;
        for (name, args) in [
            (
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
            ),
            ("agent.rename", json!({ "target": "alpha", "name": "beta" })),
            ("agent.move", json!({ "target": "alpha", "parent": "lead" })),
        ] {
            let err = call(&table, name, args.clone())
                .expect_err("must be a save refusal: {name} {args}");
            assert_eq!(err.code(), ErrorCode::Conflict, "{name}: {args}");
            assert_eq!(
                snapshot(),
                before,
                "{name}: the save refusal changed the roster — {args}"
            );
        }
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★A rename or move the store did not accept is `CONFLICT` with no notification★ — name and hierarchy are unchanged, so nothing to announce; answering `Renamed` would make the caller type the next command with a changed name.
    // ADR-0291 R17 (D3)
    #[test]
    fn a_rename_or_move_the_store_refuses_is_a_conflict_that_announces_nothing() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("lead", false, false);
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("the rename comes back as a save refusal — not Renamed");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains(PROFILES_FILE)
                && err.message().contains("the name was left as it is"),
            "{}",
            err.message()
        );

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "lead" }),
        )
        .expect_err("the move comes back as a save refusal");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(err.message().contains(PROFILES_FILE), "{}", err.message());

        assert_eq!(*notify.calls.lock().unwrap(), 0, "nothing changed");
    }

    /// ★Waking announces the roster even on a save refusal (E2)★ — the activation failure writes that entry's 「last failure」, which reaches the screen only through the roster notification (ADR-0172). The code is `CONFLICT`.
    // ADR-0291 R17 (E2)
    #[test]
    fn a_wake_the_store_refuses_is_a_conflict_that_still_announces_the_roster() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("waking comes back as a save refusal");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("could not start agent 'alpha'")
                && err.message().contains(PROFILES_FILE),
            "{}",
            err.message()
        );
        assert!(host.started.lock().unwrap().is_empty(), "nothing launched");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ★If the create of create-and-launch is refused, it does not reach the launch (§3-9 ⓐ)★ — roster unchanged · 0 launch attempts · 0 notifications; a retry creates no duplicate.
    // ADR-0291 R17
    #[test]
    fn a_create_and_start_whose_create_the_store_refuses_starts_nothing_and_announces_nothing() {
        let host = FakeHost::new();
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
        )
        .expect_err("the create comes back as a save refusal");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("nothing was registered"),
            "{}",
            err.message()
        );
        assert!(host.rows.lock().unwrap().is_empty(), "roster unchanged");
        assert!(host.started.lock().unwrap().is_empty(), "0 launch attempts");
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// ★A store refusal of the launch after creating is `INTERNAL` (§3-9 ⓑ)★ — the created agent remains, so `CONFLICT` (「rejection before touching」) would let the daemon release the request number and a same-number retry create again. The text carries the recovery path
    ///   (launch again by that name) and the store's reason only the cause — mixed-in 「again a little later」 · 「retrying is useless」 would make the caller create again with the same `--cwd` or contradict the tail. Measured per refusal cause.
    // ADR-0291 R17
    #[test]
    fn a_created_agent_whose_start_the_store_refuses_stays_internal_with_the_recovery_hint() {
        // *Defensive* — the add is stored, then the start's registry write is refused. The real manager does not write
        //   the registry after the add (`inserted` skips `register_for_spawn` · ADR-0348), so this pins the answer shape
        //   only.
        struct RefuseOnStart(Arc<FakeHost>, fn() -> StoreError);
        impl AgentCommandHost for RefuseOnStart {
            fn roster(&self) -> Vec<AgentRosterRow> {
                self.0.roster()
            }
            fn store_status(&self) -> StoreStatus {
                self.0.store_status()
            }
            fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
                self.0.agent_snapshot(id)
            }
            fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
                let stored = self.0.create_agent(profile)?;
                *self.0.store_refuses.lock().unwrap() = true;
                Ok(stored)
            }
            fn create_agent_if(
                &self,
                profile: AgentProfile,
                still: &dyn Fn() -> bool,
            ) -> Result<Option<AgentProfile>, PtyError> {
                ask_then_create(self, profile, still)
            }
            fn activate_profile(
                &self,
                profile: &AgentProfile,
                mode: SpawnMode,
            ) -> Result<StartedAgent, PtyError> {
                if *self.0.store_refuses.lock().unwrap() {
                    return Err(PtyError::Store((self.1)()));
                }
                self.0.activate_profile(profile, mode)
            }
            // The add is stored, then the start's own registry write is refused.
            fn create_and_activate(
                &self,
                profile: &AgentProfile,
                _delivery: NoticeDelivery,
            ) -> Result<Activated, NotStarted> {
                self.0
                    .create_agent(profile.clone())
                    .map_err(|error| NotStarted {
                        error,
                        created: false,
                    })?;
                Err(NotStarted {
                    error: PtyError::Store((self.1)()),
                    created: true,
                })
            }
            fn rename_agent(
                &self,
                id: AgentId,
                display_name: Option<String>,
            ) -> Result<RenameOutcome, PtyError> {
                self.0.rename_agent(id, display_name)
            }
            fn reparent_agent(
                &self,
                child: AgentId,
                parent: Option<AgentId>,
            ) -> Result<bool, PtyError> {
                self.0.reparent_agent(child, parent)
            }
            fn set_workspace(
                &self,
                id: AgentId,
                cwd: &str,
            ) -> Result<WorkspaceSet, WorkspaceRefusal> {
                self.0.set_workspace(id, cwd)
            }
            fn duplicate_agent(&self, id: AgentId) -> Result<AgentProfile, PtyError> {
                self.0.duplicate_agent(id)
            }
            fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
                self.0.list_queued_inputs(id)
            }
            fn cancel_queued_input(
                &self,
                id: AgentId,
                input_id: &str,
            ) -> Result<CancelOutcome, CancelError> {
                self.0.cancel_queued_input(id, input_id)
            }
            fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
                self.0.interrupt_agent(id)
            }
        }
        use crate::profile::Refusal;
        let causes: [(fn() -> StoreError, String); 3] = [
            (
                || StoreError::Refused(Refusal::Unreadable),
                format!("{PROFILES_FILE} could not be read just now"),
            ),
            (
                || StoreError::Refused(Refusal::Newer { found: 2 }),
                format!("{PROFILES_FILE} was written by a newer build (version 2)"),
            ),
            (
                || StoreError::Io(std::io::Error::other("disk full")),
                format!("writing {PROFILES_FILE} failed: disk full"),
            ),
        ];

        for (refusal, cause) in causes {
            let host = FakeHost::new();
            let notify = Arc::new(FakeNotify::default());
            let table = make_table(
                Arc::new(RefuseOnStart(Arc::clone(&host), refusal)) as Arc<dyn AgentCommandHost>,
                Arc::clone(&notify) as Arc<dyn RosterChanged>,
                Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
                crate::library_commands::fake::FakeLibrary::unplugged(),
            );

            let err = call(
                &table,
                "agent.spawn",
                json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
            )
            .expect_err("the launch comes back as a save refusal");

            let message = err.message();
            assert_eq!(err.code(), ErrorCode::Internal, "{message}");
            assert!(
                message.contains("was created but did not start")
                    && message.contains(&cause)
                    && message.contains(
                        "start it again by that name (in a moment) instead of creating another"
                    ),
                "{message}"
            );
            // Only the cause — the instruction of what to do is the one tail.
            assert!(
                !message.contains("retry in a moment")
                    && !message.contains("retrying will not help")
                    && !message.contains("restart the daemon"),
                "{message}"
            );
            assert_eq!(
                host.rows.lock().unwrap().len(),
                1,
                "the created agent remains"
            );
            assert!(host.started.lock().unwrap().is_empty());
            assert_eq!(
                *notify.calls.lock().unwrap(),
                1,
                "once, after the activation (E2) — the pre-activation frame is gone (ADR-0348)"
            );
        }
    }

    /// ★Through the bus with the real manager and a store that does not accept★ — the fake only imitates the `try_` versions, so whether the table really calls the manager's caller entry (an old version would stand in memory and be `Ok`) is measurable only with the real thing.
    ///   Decision refusal = `CONFLICT` · write failure = `INTERNAL`; either way roster unchanged · nothing launched · the only notification is the one after waking's activation (E2).
    /// ★It launches no process★ — the launch stops at registration (①); a regression passing through would be a shell calling a nonexistent program, ending in failure.
    // ADR-0291 R17 (D3 · E2)
    #[test]
    fn a_real_manager_behind_the_bus_changes_nothing_while_its_store_does_not_save() {
        use crate::profile::scripted_store::{Outcome, ScriptedStore, FAILURES};
        use crate::profile::ProfileRegistry;
        use crate::session_tracker::{SessionTracker, TrackerConfig};
        use crate::types::{AgentInfo, StatusSink};

        struct NoStatus;
        impl StatusSink for NoStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }
        for failure in FAILURES {
            let expected = match failure {
                Outcome::FailsIo => ErrorCode::Internal,
                _ => ErrorCode::Conflict,
            };
            let store = ScriptedStore::new();
            let manager = Arc::new(AgentManager::new(
                Arc::new(NoStatus),
                Arc::new(ProfileRegistry::new(store.clone())),
                Arc::new(SessionTracker::new(
                    TrackerConfig {
                        enabled: false,
                        poll_interval: std::time::Duration::from_secs(1),
                    },
                    Arc::new(|_, _| {}),
                )),
            ));
            let program = format!("engram-not-a-real-program-{}.exe", uuid::Uuid::new_v4());
            let shell = |name: &str| {
                let cwd = format!("C:/bus-store/{name}");
                let mut profile = AgentProfile::new(
                    cwd.clone(),
                    AgentCommand::Shell {
                        program: program.clone(),
                        args: vec![],
                    },
                    PathBuf::from(cwd),
                    vec![],
                    false,
                );
                profile.display_name = Some(name.to_string());
                profile
            };
            let alpha = manager
                .try_create_agent(shell("alpha"))
                .expect("register when saved");
            manager
                .try_create_agent(shell("lead"))
                .expect("register when saved");
            let roster = |m: &AgentManager| -> Vec<(AgentId, Option<String>, Option<AgentId>)> {
                let mut rows: Vec<_> = m
                    .agent_snapshots()
                    .into_iter()
                    .map(|p| (p.id, p.display_name, p.parent_id))
                    .collect();
                rows.sort();
                rows
            };
            let before = roster(&manager);
            store.set(failure);
            let notify = Arc::new(FakeNotify::default());
            let table = make_table(
                manager.clone() as Arc<dyn AgentCommandHost>,
                Arc::clone(&notify) as Arc<dyn RosterChanged>,
                Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
                crate::library_commands::fake::FakeLibrary::unplugged(),
            );

            for (name, args) in [
                (
                    "agent.new",
                    json!({ "cwd": "C:/bus-store/delta", "backend": "Claude" }),
                ),
                ("agent.rename", json!({ "target": "alpha", "name": "beta" })),
                ("agent.move", json!({ "target": "alpha", "parent": "lead" })),
                ("agent.spawn", json!({ "cwd": "C:/bus-store/gamma" })),
                ("agent.spawn", json!({ "target": "alpha" })),
            ] {
                let err = call(&table, name, args.clone())
                    .expect_err("an error if the store does not accept: {name} {args}");
                assert_eq!(err.code(), expected, "{failure:?} · {name} {args}");
                assert!(
                    err.message().contains(PROFILES_FILE),
                    "{failure:?} · {name}: {}",
                    err.message()
                );
            }

            assert_eq!(roster(&manager), before, "{failure:?}: roster unchanged");
            assert!(
                manager.list_agents().is_empty(),
                "{failure:?}: nothing launched"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                1,
                "{failure:?}: only the notification after waking's activation (E2)"
            );
            assert_eq!(
                manager
                    .agent_snapshot(alpha.id)
                    .and_then(|p| p.last_failure),
                Some(crate::failure::AgentFailureKind::Other),
                "{failure:?}: recorded as 'other', not as a launch failure"
            );
        }
    }

    /// ★Pins **the value itself** of [`NEW_AGENT_OUTPUT_FORMAT`] as a literal★ — every reader of the constant compares relatively (`assert_eq!(…, NEW_AGENT_OUTPUT_FORMAT)`), so flipping it to `Terminal` moves those assertions along and **the whole workspace stays green**
    /// (no literal assertion existed). Relative comparison measures 「both branches look at the same home」, not 「what is in it」 — the latter is what the user sees (chat or terminal after creation). Changing the value is a decision (ADR-0078).
    #[test]
    fn the_new_agent_default_output_format_is_pinned_to_a_literal() {
        assert_eq!(NEW_AGENT_OUTPUT_FORMAT, CoreAgentOutputFormat::StreamJson);
    }

    /// The value list of that field the `agent.new` declaration advertised — **the list callers actually see**. If the test wrote this by hand,
    /// it would become a third list and create the very divergence it meant to measure.
    fn advertised(field: &str) -> Vec<String> {
        let schema: serde_json::Value =
            serde_json::from_str(AgentNewArgs::SPEC.args_schema).expect("args schema");
        // ★Both shapes are read★ — a required field spreads as `{"enum":[…]}`, an omittable one is wrapped as
        //   `{"anyOf":[{"enum":[…]},…]}`; reading one shape would make the sweep an **empty list** when the field switches
        //   required/optional, silently measuring nothing
        //   (the caller's `is_empty` assertion is the last defense).
        let property = &schema["properties"][field];
        let branches = match property.get("anyOf") {
            Some(any_of) => any_of.as_array().expect("anyOf is an array").clone(),
            None => vec![property.clone()],
        };
        branches
            .iter()
            .filter_map(|branch| branch.get("enum"))
            .flat_map(|values| values.as_array().expect("enum array").clone())
            .map(|value| value.as_str().expect("string").to_string())
            .collect()
    }

    /// ★The place where **the compiler** binds the declaration vocabulary and the core vocabulary★ — prose binding would leave the declaration silently narrow when the core gains a variant, and that word would be rejected at the entry with `INVALID_ARGUMENT` (no error or log).
    ///
    /// ★Half is **compilation**★ — the two `match`es below have no wildcard, so a new core variant **fails to compile** here; do not put `_ =>` when filling branches.
    /// ★The other half comes from **the value list the declaration advertised**★ — hand-written words would let an edit adding a variant but not the list bypass the spelling check. Drawn from the advertisement: core gains a variant → `match` breaks → variant added to the
    /// declaration → enters the advertisement → this sweep covers it.
    #[test]
    fn the_declared_vocabularies_are_pinned_to_the_core_ones() {
        // ── Output format: declaration variants ↔ core variants are one-to-one, down to the wire notation ──
        fn declared_output_format(core: CoreAgentOutputFormat) -> AgentOutputFormat {
            match core {
                CoreAgentOutputFormat::Terminal => AgentOutputFormat::Terminal,
                CoreAgentOutputFormat::StreamJson => AgentOutputFormat::StreamJson,
            }
        }
        let formats = advertised("output_format");
        assert!(
            !formats.is_empty(),
            "if the advertisement is empty, this sweep is disarmed"
        );
        for word in &formats {
            let spelled = serde_json::Value::String(word.clone());
            // The same word must be read **on both sides** — if only one side reads it, that place is exactly the point of divergence.
            let declared: AgentOutputFormat = serde_json::from_value(spelled.clone())
                .expect("the declaration vocabulary reads the advertisement");
            let core: CoreAgentOutputFormat =
                serde_json::from_value(spelled).expect("the core vocabulary reads the same word");
            assert_eq!(
                declared_output_format(core),
                declared,
                "{word}: the two vocabularies read the same word as different variants"
            );
            assert_eq!(
                output_format(Some(declared)),
                core,
                "{word}: the declaration → core mapping must be one-to-one"
            );
        }

        // ── Backends: for each run command variant of the core, decide here **whether this entry can create it** ──
        fn declared_backend(command: &AgentCommand) -> Option<AgentBackend> {
            match command {
                AgentCommand::Claude { .. } => Some(AgentBackend::Claude),
                // ★`Shell` being `None` is a decision, not a blank★ — this entry cannot create it
                //   (it has not been confirmed that the profile creation path runs with it — the `AgentBackend` declaration).
                AgentCommand::Shell { .. } => None,
                // ★`Codex` is open★ — its old `None` reason (trust modal) was overturned by measurement (grounds: the codex line of
                //   [`LLM_BACKEND_POLICY`]). This line now measures **the vocabulary round trip**: whether the run command the declaration
                //   `Codex` builds reads back as `Codex`
                //   and the stored `kind` tag has the same spelling.
                AgentCommand::Codex { .. } => Some(AgentBackend::Codex),
            }
        }
        let backends = advertised("backend");
        assert!(
            !backends.is_empty(),
            "if the advertisement is empty, this sweep is disarmed"
        );
        for word in &backends {
            let declared: AgentBackend =
                serde_json::from_value(serde_json::Value::String(word.clone()))
                    .expect("the declaration vocabulary reads the advertisement");
            let built = backend_command(declared.clone(), NEW_AGENT_OUTPUT_FORMAT);
            assert_eq!(
                declared_backend(&built),
                Some(declared),
                "{word}: the run command the declaration vocabulary builds does not read back into that vocabulary"
            );
            // ★Binds the core-side wire spelling too★: `AgentCommand` is written to disk as is with `#[serde(tag = "kind")]`
            //   (the profile file), so changing that tag value must be caught here — otherwise the declared vocabulary and the
            //   stored vocabulary diverge silently, and the divergence shows up as an old build unable to read the file.
            assert_eq!(
                serde_json::to_value(&built).expect("core serialization")["kind"],
                serde_json::Value::String(word.clone()),
                "{word}: the declaration vocabulary and the stored kind tag diverged"
            );
        }
    }

    /// ★The values passing today are `Claude`·`Codex`, **each working in both modes**★.
    ///
    /// The core also has `Shell`, but the creation path is unconfirmed with it, so it is outside the vocabulary (reason: the `AgentBackend` declaration).
    ///
    /// ★The mode axis is measured here too★ — opening codex is **word × two modes**, and whether the mode was carried is visible only in **the stored run command** (`AgentNewOk` has no such field); otherwise the suite would stay green even if [`backend_command`] pinned codex
    /// to one mode.
    #[test]
    fn new_accepts_both_declared_backends_and_refuses_the_rest() {
        for word in ["Claude", "Codex"] {
            for (spelled, expected) in [
                ("Terminal", CoreAgentOutputFormat::Terminal),
                ("StreamJson", CoreAgentOutputFormat::StreamJson),
            ] {
                let host = FakeHost::new();
                let (table, _notify) = wiring(&host);

                call(
                    &table,
                    "agent.new",
                    json!({ "cwd": "C:/x", "backend": word, "output_format": spelled }),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "{word}/{spelled}: the declaration vocabulary was rejected — {}",
                        e.message()
                    )
                });

                let profiles = host.profiles.lock().unwrap();
                let stored = profiles.values().next().expect("one registered profile");
                let (kind, format) = match &stored.command {
                    AgentCommand::Claude {
                        extra_args,
                        output_format,
                    } => ("Claude", (extra_args.clone(), *output_format)),
                    AgentCommand::Codex {
                        extra_args,
                        output_format,
                    } => ("Codex", (extra_args.clone(), *output_format)),
                    other => panic!("{word}/{spelled}: the wrong run command — {other:?}"),
                };
                assert_eq!(
                    kind, word,
                    "{word}/{spelled}: a different backend was stored"
                );
                // ★This entry does not grow the argument surface★ — if fields like model or reasoning effort leaked in here, the assertion
                //   that it is empty
                //   breaks first.
                assert!(
                    format.0.is_empty(),
                    "{word}/{spelled}: this entry carries no extra_args — {:?}",
                    format.0
                );
                assert_eq!(
                    format.1, expected,
                    "{word}/{spelled}: the mode was not carried"
                );
            }
        }

        // ★Outside the vocabulary is rejected before registration — but **spelling is not the outside-vocabulary axis**★: a word differing
        //   only in case
        //   reads as the same word (`new_accepts_the_backend_word_in_any_casing`).
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);
        for outside in ["Gemini", "Shell", "codx"] {
            let err = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/x", "backend": outside }),
            )
            .expect_err("a backend outside the vocabulary");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{outside}");
            // It must say **everything** that works — saying only one would make the guidance fall behind the day the vocabulary widens.
            for accepted in ["Claude", "Codex"] {
                assert!(
                    err.message().contains(accepted),
                    "{outside}: does not say the working word '{accepted}' — {}",
                    err.message()
                );
            }
        }
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★Not giving the field at all is a rejection — it does not silently become claude★. Unspecified used to mean `Claude`; with two backends that default folds 「did not choose」 and 「chose claude」 into one value (already removed from the spawn packet — the daemon's
    /// `MISSING_BACKEND`), and leaving only this door would split that decision per entry.
    #[test]
    fn new_refuses_an_unspecified_backend_instead_of_defaulting() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.new", json!({ "cwd": "C:/x" }))
            .expect_err("not choosing is not choosing claude");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("backend"),
            "must say which field is empty: {}",
            err.message()
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★This door opens **the same set** as [`LLM_BACKEND_POLICY`] — measured by **calls**, not by comparing lists★.
    ///
    /// List matching stays green even with the gate removed entirely (it happened at a sibling door). So for each table word `agent.new` is **called**: words the table opened go onto the roster, words it closed do not **in any spelling**.
    ///
    /// ★The spellings this door accepts are decided by the declaration vocabulary★ — they differ from the table's words only in case ([`LlmBackendPolicy`] `word` doc), so **every spelling this door could accept** is tried; one spelling would silently pass the day the vocabulary widens.
    ///
    /// ★What it catches★:
    ///   - **table closed but this door still creates** → red: an edit widening the vocabulary ([`AgentBackend`]) while deleting only the [`llm_creation_refusal`] arm of `verb_new`. The only regression net of that arm.
    ///   - **table opened but this door does not create** → red: an edit adding a table line without filling the vocabulary and [`backend_command`] (guidance in the [`LLM_BACKEND_POLICY`] doc).
    ///
    /// ★**Deleting only the arm** is uncatchable here even today★ — the table closes nothing, so no input reaches the arm. Do not read that as 「the arm is unnecessary」: the day the table closes a line again it is the only runtime defense.
    #[test]
    fn new_creates_exactly_what_the_llm_backend_policy_opens() {
        /// Spelling candidates this door can accept for this word — the table is lowercase, the declaration vocabulary PascalCase.
        fn spellings(word: &str) -> Vec<String> {
            let mut chars = word.chars();
            let pascal = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            };
            if pascal == word {
                vec![word.to_string()]
            } else {
                vec![word.to_string(), pascal]
            }
        }

        assert!(
            !LLM_BACKEND_POLICY.is_empty(),
            "if the table is empty, this sweep is disarmed"
        );
        for policy in LLM_BACKEND_POLICY {
            let mut opened_by_some_spelling = false;
            for spelling in spellings(policy.word) {
                let host = FakeHost::new();
                let (table, notify) = wiring(&host);
                let outcome = call(
                    &table,
                    "agent.new",
                    json!({ "cwd": "C:/x", "backend": spelling }),
                );
                match outcome {
                    Ok(_) => {
                        opened_by_some_spelling = true;
                        assert_eq!(
                            host.rows.lock().unwrap().len(),
                            1,
                            "{spelling}: passed but did not go onto the roster"
                        );
                    }
                    Err(err) => {
                        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{spelling}");
                        assert!(
                            host.rows.lock().unwrap().is_empty(),
                            "{spelling}: ★refused before registration★"
                        );
                        assert_eq!(*notify.calls.lock().unwrap(), 0, "{spelling}");
                    }
                }
            }
            assert_eq!(
                opened_by_some_spelling,
                policy.refusal.is_none(),
                "'{}': the set this door opens and the policy table diverged — the table {}, this door {}",
                policy.word,
                if policy.refusal.is_none() {
                    "opens"
                } else {
                    "closes"
                },
                if opened_by_some_spelling {
                    "creates"
                } else {
                    "does not create"
                }
            );
        }
    }

    /// ★The backend words `agent.new` **advertises** == the set the policy table opens★ (spellings differ only in case: declaration `Claude` · table `claude`).
    ///
    /// `new_creates_exactly_what_the_llm_backend_policy_opens` sweeps **the table's lines** and cannot see a word only in the advertisement — a variant added to the vocabulary without a table line is advertised but closed by [`NO_POLICY_DECLARED`], rejecting a value the caller
    /// was told it could choose. This catches that direction.
    #[test]
    fn new_advertises_exactly_the_backends_the_llm_policy_opens() {
        let mut opened: Vec<String> = LLM_BACKEND_POLICY
            .iter()
            .filter(|policy| policy.refusal.is_none())
            .map(|policy| policy.word.to_ascii_lowercase())
            .collect();
        let mut advertised: Vec<String> = advertised("backend")
            .iter()
            .map(|word| word.to_ascii_lowercase())
            .collect();
        assert!(
            !advertised.is_empty(),
            "if the advertisement is empty, this test is disarmed"
        );
        opened.sort();
        advertised.sort();
        assert_eq!(
            advertised, opened,
            "the `AgentBackend` vocabulary of `agent.new` and the set `LLM_BACKEND_POLICY` opens diverged"
        );
    }

    #[test]
    fn rename_splits_renamed_from_unchanged() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Unchanged("alpha".to_string()));
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "alpha" }),
        )
        .expect("rename");
        assert_eq!(out["outcome"], "unchanged");

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("rename");
        assert_eq!(out["outcome"], "renamed");
        assert_eq!(out["name"], "beta");
        assert_eq!(
            *notify.calls.lock().unwrap(),
            2,
            "both outcomes change the roster display, so they announce"
        );
    }

    #[test]
    fn rename_exhaustion_is_a_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Exhausted);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("name space exhausted");
        assert_eq!(err.code(), ErrorCode::Conflict);
    }

    /// ★`null` is an active instruction to 「detach to root」, and **absence is rejected**★ — folding them would make one typo field
    /// silently run a hierarchy release.
    #[test]
    fn move_splits_an_absent_parent_from_an_explicit_null() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("null = to top level");
        assert_eq!(out["parent"], serde_json::Value::Null);
        assert_eq!(
            out["name"], "alpha",
            "the success response carries the name"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "announces the roster change"
        );

        let err = call(&table, "agent.move", json!({ "target": "alpha" }))
            .expect_err("absence is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("parent"),
            "says which field is missing: {}",
            err.message()
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "a rejection does not notify"
        );
    }

    #[test]
    fn a_structural_rejection_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("the tree structure refuses");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// ★A single `false` is not read as a reason★ — vanished in the meantime and a structural conflict have different things for the caller to do.
    #[test]
    fn a_target_that_vanished_mid_move_is_not_found() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(alpha);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect_err("the target vanished");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(err.message().contains("alpha"), "{}", err.message());
    }

    /// ★Name the side that vanished★ — if only the parent disappeared but the child's name were given, the caller would hear that an agent that is perfectly there
    /// is missing and search in the wrong place.
    #[test]
    fn a_parent_that_vanished_mid_move_names_the_parent_not_the_child() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let beta = host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(beta);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("the new parent vanished");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().contains("beta") && !err.message().contains("alpha"),
            "must point at the side that vanished: {}",
            err.message()
        );
    }

    #[test]
    fn unknown_target_is_not_found_and_ambiguous_target_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("twin", false, false);
        host.with_agent("twin", false, false);
        let (table, _notify) = wiring(&host);

        let missing =
            call(&table, "agent.spawn", json!({ "target": "nope" })).expect_err("a missing name");
        assert_eq!(missing.code(), ErrorCode::NotFound);

        let ambiguous = call(&table, "agent.spawn", json!({ "target": "twin" }))
            .expect_err("two with the same name");
        assert_eq!(ambiguous.code(), ErrorCode::Conflict);
    }

    #[test]
    fn an_id_is_matched_before_a_name() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": id.to_string() })).expect("id pointing");
        assert_eq!(host.started.lock().unwrap()[0].0, id);
    }

    #[test]
    fn a_blank_value_is_refused_not_treated_as_absent() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "   " }),
        )
        .expect_err("a whitespace-only value");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        // ★For the same field, whichever mistake was made, the same branch is told★: if the blank-value text hid the `null` (detach to top
        //   level)
        //   the absence text tells about, the caller would read that putting a value into that field is the only way.
        assert!(
            err.message().contains("null"),
            "must tell the null branch for a blank value too: {}",
            err.message()
        );
    }

    /// ★Doing as the rejection text says must work★: telling a caller who sent a required field blank "may be omitted" would get the omitting retry **rejected again** (as 「missing」) and the LLM circles the same place; a required field is told only to fill in a value.
    #[test]
    fn the_advice_for_a_blank_required_field_does_not_tell_the_caller_to_drop_it() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let required = call(
            &table,
            "agent.new",
            json!({ "cwd": "   ", "backend": "Claude" }),
        )
        .expect_err("the field deciding where to create is blank")
        .message()
        .to_string();
        assert!(
            required.contains("cwd needs a real value"),
            "must say to fill the value: {required}"
        );
        assert!(
            !required.contains("left out"),
            "saying to omit it gets the retry rejected again as 「missing」: {required}"
        );
        // ★A rejection creates nothing★ — if the check were pushed after registration, a call with blank arguments would create an agent
        //   and then
        //   answer with a rejection (the caller reads it as a failure and calls again — only the names grow one by one).
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // An optional field is the opposite — omitting it is the way that actually works, so that way is told too.
        let optional = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": " ", "backend": "Claude" }),
        )
        .expect_err("an optional field is blank")
        .message()
        .to_string();
        assert!(
            optional.contains("name") && optional.contains("left out"),
            "saying an optional field may be omitted is true: {optional}"
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // The retry that omitted it as told actually works.
        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "backend": "Claude" }),
        )
        .expect("following the omit guidance works");
    }

    /// ★Both fields of `rename` are **required**★ — telling either 「may be omitted」 would get the omitting retry rejected at the entry check with 「requires …」.
    ///
    /// ★A property to look at **per call site**, not per field★: whether blank-value guidance is right depends on what that field must be in that verb, so checking the shared function once guards no verb.
    #[test]
    fn a_blank_rename_argument_is_told_to_fill_it_not_to_drop_it() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        for (args, blank) in [
            (json!({ "target": "alpha", "name": "  " }), "name"),
            (json!({ "target": " ", "name": "beta" }), "target"),
        ] {
            let refusal = call(&table, "agent.rename", args.clone())
                .expect_err("blank value")
                .message()
                .to_string();

            assert!(
                refusal.contains(&format!("{blank} needs a real value")),
                "must say to fill in {blank}: {refusal}"
            );
            assert!(
                !refusal.contains("left out"),
                "saying to omit {blank} gets that retry rejected again with requires: {refusal}"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "a rejection does not notify"
            );
        }

        // The retry that filled both fields as the text said works.
        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("filling in as guided works");
        assert_eq!(out["name"], "beta", "{out}");
    }

    /// ★The constraint of `spawn` is **a combination**, not a field★: `cwd` is `Option` by declaration so 「may be omitted」 is true there, but omitting it is rejected again with 「either target or cwd must be given」; the text must state the combination in one go.
    #[test]
    fn a_blank_spawn_argument_is_told_what_actually_satisfies_the_verb() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);
        let before = host.rows.lock().unwrap().len();

        let refusal = call(&table, "agent.spawn", json!({ "cwd": "  " }))
            .expect_err("blank cwd")
            .message()
            .to_string();

        assert!(
            refusal.contains("cwd"),
            "must point out which field: {refusal}"
        );
        assert!(
            !refusal.contains("left out"),
            "saying to omit it gets that retry rejected again: {refusal}"
        );
        assert!(
            refusal.contains("target") && refusal.contains("cwd"),
            "must say what to give for it to work (one of the two): {refusal}"
        );
        // ★In this verb the rejection must come **before anything irreversible**★: a blank-value check after the combination branch would
        //   let a blank `cwd` **create an agent and notify** and then answer with a rejection; the caller reads a failure, calls again,
        //   names grow
        //   one by one, and the created ones cannot be deleted (ADR-0122).
        assert_eq!(
            host.rows.lock().unwrap().len(),
            before,
            "the rejection created an agent"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // **Both** ways the text points at actually work.
        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("the wake way");
        call(&table, "agent.spawn", json!({ "cwd": "C:/work/new" })).expect("the create way");
    }

    /// ★`move` points out the two blank fields **at once**★ — rejecting only `parent` first would make the caller hear about `target` only on the next trip after fixing it
    /// (two round trips for one mistyped call). And the `null` branch of `parent` still remains.
    #[test]
    fn a_move_with_two_blank_fields_names_both_in_one_trip() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let refusal = call(
            &table,
            "agent.move",
            json!({ "target": " ", "parent": "  " }),
        )
        .expect_err("both blank")
        .message()
        .to_string();

        assert!(
            refusal.contains("target"),
            "the sibling field was hidden: {refusal}"
        );
        assert!(refusal.contains("parent"), "{refusal}");
        assert!(
            refusal.contains("null"),
            "points out at once without losing the null branch: {refusal}"
        );

        // The retry fixed as the text said works.
        call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("fixing as guided works");
    }

    /// ★With several blank fields, all of them come out★ — pointing one at a time would make a caller who sent three unset shell variables repeat rejection and fix
    /// three times (the same discipline as the entry's unknown-field check).
    #[test]
    fn every_blank_field_is_named_not_just_the_first() {
        let (table, _notify) = wiring(&FakeHost::new());

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": " ", "cwd": "  ", "name": "\t" }),
        )
        .expect_err("all blank");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        for field in ["target", "cwd", "name"] {
            assert!(
                err.message().contains(field),
                "{field} is missing: {}",
                err.message()
            );
        }
    }

    // ── State honesty ──────────────────────────────────────────────────────────────

    /// ★The change verbs and `list` must pass through the same function so the two answers cannot diverge★ — every state is pinned here.
    ///
    /// ★Known blind spot (not fixed)★: the list below is **hand-written** and `state_of` emits asleep through a catch-all (`_ =>`), so a new 「live-natured」 variant would silently go out as asleep with this test green; filling the list automatically needs a shape change on the
    /// `AgentStatus` side.
    #[test]
    fn every_status_maps_to_one_of_the_two_agent_state_words() {
        assert_eq!(
            state_of(None),
            AGENT_STATE_SLEEPING,
            "no session means asleep"
        );
        assert_eq!(state_of(Some(&AgentStatus::Running)), AGENT_STATE_LIVE);
        assert_eq!(
            state_of(Some(&AgentStatus::Exiting)),
            AGENT_STATE_LIVE,
            "going down is still alive"
        );
        for dead in [
            AgentStatus::Exited { code: Some(0) },
            AgentStatus::Exited { code: None },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "boom".to_string(),
            },
        ] {
            assert_eq!(
                state_of(Some(&dead)),
                AGENT_STATE_SLEEPING,
                "a corpse is not alive: {dead:?}"
            );
        }
        // Not mixed with the message outcome vocabulary (mixing the two axes once tangled decisions — ADR-0116).
        for message_word in ["delivered", "pending", "failed"] {
            assert_ne!(AGENT_STATE_LIVE, message_word);
            assert_ne!(AGENT_STATE_SLEEPING, message_word);
        }
    }

    /// ★The response state is **derived**, not a literal★: if `spawn` answered alive for an agent that died right after launching (ADR-0082 resume early exit — no fresh fallback) the next `list` would say asleep and the caller would write mail to a corpse. Baking `"live"` into
    /// `started_payload` dies here.
    #[test]
    fn a_spawn_whose_activation_lands_terminal_reports_sleeping() {
        for terminal in [
            AgentStatus::Exited { code: Some(1) },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "resume died".to_string(),
            },
        ] {
            let host = FakeHost::new();
            host.with_agent("alpha", false, true);
            *host.started_status.lock().unwrap() = Some(terminal.clone());
            let (table, _notify) = wiring(&host);

            let woken = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
            assert_eq!(
                woken["state"], AGENT_STATE_SLEEPING,
                "something that died right after waking must not be reported as alive: {terminal:?}"
            );
            assert_eq!(woken["created"], false);

            let born = call(&table, "agent.spawn", json!({ "cwd": "C:/work/beta" }))
                .expect("create and launch");
            assert_eq!(
                born["state"], AGENT_STATE_SLEEPING,
                "the create path uses the same derivation: {terminal:?}"
            );
            assert_eq!(born["created"], true);
        }
    }

    /// Terminal states held in the roster come out as the same words — the other half of the axis that change verbs and queries pass through one function.
    #[test]
    fn the_roster_reports_a_terminal_status_as_sleeping() {
        let host = FakeHost::new();
        host.with_status(
            "zombie",
            Some(AgentStatus::Failed {
                message: "died".to_string(),
            }),
            false,
        );
        host.with_status("alive", Some(AgentStatus::Running), false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("query");
        let agents = out["agents"].as_array().expect("array");
        assert_eq!(agents[0]["state"], AGENT_STATE_SLEEPING, "{out}");
        assert_eq!(agents[1]["state"], AGENT_STATE_LIVE, "{out}");
    }

    /// ★The name of the move response is the value **after** applying★ — carrying the resolve-time value would, because of a rename committed in between, make the response
    /// 「old name + new parent」, a combination that never existed, and the caller would type the next command with that old name.
    #[test]
    fn move_reports_the_name_the_agent_has_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        let lead = host.with_agent("lead", false, false);
        // ★The old parent is planted★: the only way to tell whether the response's `parent` is **the parent this call instructed** or the
        //   old value left in the roster (otherwise both carry the same value). ★The fake manager does not fix the parent field★, so the
        //   old value stays.
        let stale = AgentId::new_v4();
        for row in host.rows.lock().unwrap().iter_mut() {
            if row.id == helper {
                row.parent = Some(stale);
            }
        }
        *host.rename_on_reparent.lock().unwrap() = Some("renamed".to_string());
        let (table, _notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": "lead" }),
        )
        .expect("move");
        assert_eq!(out["name"], "renamed", "{out}");
        assert_eq!(
            out["parent"],
            lead.to_string(),
            "carries the parent this call attached (not the old parent left in the roster): {out}"
        );
    }

    /// ★A target applied but vanished in the meantime★ — the response must not become **a nameless success row** (an empty name would make the caller's decider read the success as "an unreadable response" and record a move that happened as a failure).
    ///
    /// ★Resolution must succeed to reach this branch★: an emptied roster would make `resolve` emit `NOT_FOUND` first, so the vanishing is caused **at apply time** (`vanish_on_reparent`).
    #[test]
    fn move_falls_back_to_the_resolved_name_when_the_agent_vanishes_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        // The apply succeeds (reparent_ok = true) but within that call it vanishes from the roster.
        *host.vanish_on_reparent.lock().unwrap() = Some(helper);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": null }),
        )
        .expect("the apply itself succeeded");

        // ★Without this assertion the test does not exercise the fallback★: if the row remained, `current_name` would give a value and
        //   the name assertion below would pass as is — that is, green even if the seam did not fire.
        assert!(
            host.roster().iter().all(|row| row.id != helper),
            "premise: it must have vanished from the roster during the apply for the fallback to run"
        );
        assert_eq!(
            out["name"], "helper",
            "if it cannot be read after applying, fall back to the resolve-time name (no empty name): {out}"
        );
        assert_eq!(out["agent_id"], helper.to_string(), "{out}");
        assert!(
            out["name"].as_str().is_some_and(|n| !n.is_empty()),
            "a nameless row must not be carried in a success response: {out}"
        );
        // Even if the target vanished, **the parent axis carries the value this call instructed** — an empty or missing `parent` would
        //   leave the caller unable to read what was applied. (The old-parent defect splits only while the roster remains; the neighboring
        //   test looks at it.)
        assert!(
            out["parent"].is_null(),
            "it was told to detach, so null: {out}"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "it was applied, so it announces"
        );
    }

    // ── Pointing resolution (the function the cross-check runs) ────────────────────────────────────────

    /// ★This is the entry point the daemon's cross-check test runs★ — that this is the same function as the resolver the table's verbs use is
    /// the premise of that check (`resolve` delegates to this function).
    #[test]
    fn resolve_in_matches_ids_first_then_exact_names_and_refuses_duplicates() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        let twin_a = host.with_agent("twin", false, false);
        let _twin_b = host.with_agent("twin", false, false);
        let roster = host.roster();

        assert_eq!(resolve_in(&roster, "alpha").expect("name match").id, alpha);
        assert_eq!(
            resolve_in(&roster, &twin_a.to_string())
                .expect("id match")
                .id,
            twin_a,
            "id pointing works even with the same name"
        );
        assert_eq!(
            resolve_in(&roster, "twin")
                .expect_err("two with the same name")
                .code(),
            ErrorCode::Conflict
        );
        for miss in ["ALPHA", "alph", " alpha", "alpha "] {
            assert_eq!(
                resolve_in(&roster, miss)
                    .expect_err("exact match only")
                    .code(),
                ErrorCode::NotFound,
                "{miss}"
            );
        }
    }

    /// ★Ids are accepted in every spelling `Uuid` reads — before names★: even if another agent carries the uppercase spelling as its name, the id
    /// wins. An id-shaped value not in the roster falls through to the name and is absent.
    #[test]
    fn resolve_in_reads_every_uuid_spelling_as_the_id_before_names() {
        let alpha = AgentId::new_v4();
        let upper = alpha.to_string().to_uppercase();
        let row = |id: AgentId, name: &str| AgentRosterRow {
            id,
            canonical_name: name.to_string(),
            cwd: String::new(),
            parent: None,
            live: None,
        };
        let roster = vec![row(alpha, "alpha"), row(AgentId::new_v4(), &upper)];

        for spelling in [
            upper.clone(),
            format!("{{{alpha}}}"),
            format!("{{{upper}}}"),
            alpha.simple().to_string(),
            alpha.simple().to_string().to_uppercase(),
            alpha.urn().to_string(),
        ] {
            assert_eq!(
                resolve_in(&roster, &spelling).expect("id spelling").id,
                alpha,
                "{spelling}"
            );
        }
        let ghost = AgentId::new_v4().to_string().to_uppercase();
        assert_eq!(
            resolve_in(&roster, &ghost)
                .expect_err("an id not in the roster")
                .code(),
            ErrorCode::NotFound
        );
        assert_eq!(
            resolve_in(&roster, &format!(" {alpha}"))
                .expect_err("no whitespace correction")
                .code(),
            ErrorCode::NotFound
        );
    }

    // ── Candidates of pointing failures (TRD S28 §6) ───────────────────────────────────────────────

    /// Plants name lines into the roster as is — candidates look only at roster rows, so no profile is needed.
    fn roster_of(names: &[&str]) -> Vec<AgentRosterRow> {
        names
            .iter()
            .map(|name| AgentRosterRow {
                id: AgentId::new_v4(),
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live: None,
            })
            .collect()
    }

    // ADR-0307
    #[test]
    fn a_missing_name_lists_the_names_that_contain_it_ignoring_case() {
        let roster = roster_of(&["alpha-main", "beta", "ALPHA-two", "gamma"]);

        let err = resolve_in(&roster, "alpha").expect_err("there is no exact match");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().starts_with(&not_found_message("alpha")),
            "the old text stays in front as is: {}",
            err.message()
        );
        // In name order (byte order — uppercase first).
        assert!(
            err.message()
                .ends_with(" — candidates: ALPHA-two, alpha-main"),
            "{}",
            err.message()
        );
    }

    /// Without containing names, the first five of all in name order — the same name once.
    // ADR-0307
    #[test]
    fn with_no_containing_name_the_first_five_distinct_names_by_name_are_listed() {
        let roster = roster_of(&["f", "b", "d", "b", "a", "e", "c"]);

        let err = resolve_in(&roster, "zzz").expect_err("absent");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().ends_with(" — candidates: a, b, c, d, e"),
            "{}",
            err.message()
        );
    }

    /// ★The order of roster rows does not change the candidates★ — asleep rows of the production roster come in HashMap order.
    // ADR-0307
    #[test]
    fn candidates_do_not_depend_on_roster_row_order() {
        let forward = roster_of(&["g", "f", "e", "d", "c", "b", "a", "twin", "twin"]);
        let mut backward: Vec<AgentRosterRow> = forward
            .iter()
            .map(|row| AgentRosterRow {
                id: row.id,
                canonical_name: row.canonical_name.clone(),
                cwd: row.cwd.clone(),
                parent: None,
                live: None,
            })
            .collect();
        backward.reverse();

        for token in ["zzz", "twin"] {
            assert_eq!(
                resolve_in(&forward, token).expect_err("failure").message(),
                resolve_in(&backward, token).expect_err("failure").message(),
                "{token}"
            );
        }
    }

    /// An empty roster gives `none` — without the tail, 「no candidates」 and 「a text that carries no candidates」 look the same.
    #[test]
    fn an_empty_roster_says_there_are_no_candidates() {
        let err = resolve_in(&[], "alpha").expect_err("absent");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert_eq!(
            err.message(),
            format!("{} — candidates: none", not_found_message("alpha"))
        );
    }

    /// Candidates for the same name are ids, not names — giving the name again would be the same CONFLICT again.
    // ADR-0307
    #[test]
    fn a_duplicate_name_lists_the_clashing_ids_up_to_five() {
        let roster = roster_of(&["other", "twin", "twin"]);
        let err = resolve_in(&roster, "twin").expect_err("two with the same name");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message()
                .starts_with("more than one agent is called 'twin'"),
            "{}",
            err.message()
        );
        let mut pair = [roster[1].id.to_string(), roster[2].id.to_string()];
        pair.sort();
        assert!(
            err.message()
                .ends_with(&format!(" — candidates: {}, {}", pair[0], pair[1])),
            "{}",
            err.message()
        );

        let crowd = roster_of(&["twin"; 7]);
        let err = resolve_in(&crowd, "twin").expect_err("seven with the same name");
        let mut ids: Vec<String> = crowd.iter().map(|row| row.id.to_string()).collect();
        ids.sort();
        ids.truncate(5);
        assert!(
            err.message()
                .ends_with(&format!(" — candidates: {}", ids.join(", "))),
            "{}",
            err.message()
        );
    }

    /// Candidates come from the one resolver, so every verb taking `target` receives them together.
    #[test]
    fn a_target_verb_carries_the_candidates_too() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alp", "name": "x" }),
        )
        .expect_err("a missing name");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().ends_with(" — candidates: alpha"),
            "{}",
            err.message()
        );
    }

    // ── agent.resolve ────────────────────────────────────────────────────────────

    // ADR-0307
    #[test]
    fn resolve_answers_every_target_in_order_with_name_id_and_liveness() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", true, false);
        let beta = host.with_agent("beta", false, false);
        let corpse = host.with_status("gamma", Some(AgentStatus::Killed), false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["beta", alpha.to_string(), "gamma", "alpha"] }),
        )
        .expect("all resolve");
        assert_eq!(
            out,
            json!({ "agents": [
                { "target": "beta", "id": beta.to_string(), "name": "beta", "live": false },
                { "target": alpha.to_string(), "id": alpha.to_string(), "name": "alpha", "live": true },
                { "target": "gamma", "id": corpse.to_string(), "name": "gamma", "live": false },
                { "target": "alpha", "id": alpha.to_string(), "name": "alpha", "live": true },
            ]})
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a query does not change the roster"
        );
    }

    /// ★`live` is one decision with the `state` of `agent.list`★ — even if the session remains, an ended incarnation (not yet collected by the reaper)
    /// is not alive (ADR-0116).
    #[test]
    fn resolve_and_list_agree_on_liveness() {
        let host = FakeHost::new();
        host.with_status("exiting", Some(AgentStatus::Exiting), false);
        host.with_status("exited", Some(AgentStatus::Exited { code: Some(0) }), false);
        host.with_agent("asleep", false, false);
        let (table, _notify) = wiring(&host);

        let listed = call(&table, "agent.list", json!({})).expect("query");
        let resolved = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["exiting", "exited", "asleep"] }),
        )
        .expect("query");
        for (row, answer) in listed["agents"]
            .as_array()
            .expect("array")
            .iter()
            .zip(resolved["agents"].as_array().expect("array"))
        {
            assert_eq!(
                answer["live"].as_bool(),
                Some(row["state"] == AGENT_STATE_LIVE),
                "{row} / {answer}"
            );
        }
    }

    /// Exactly the form the shell reads — `targets[` + decimal position + `]: `. `None` if the form does not match.
    fn failing_position(message: &str) -> Option<usize> {
        let rest = message.strip_prefix("targets[")?;
        let (digits, after) = rest.split_once(']')?;
        after.strip_prefix(": ")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }

    /// ★A failure caused by one word keeps its code and its text starts with `targets[i]: ` — the shell reads this head★ (the declaration header comment). With several blank words the head is the first position.
    // ADR-0307
    #[test]
    fn the_resolve_failure_prefix_is_a_stable_contract() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        host.with_agent("twin", false, false);
        host.with_agent("twin", false, false);
        let (table, _notify) = wiring(&host);
        let fail = |targets: serde_json::Value| {
            call(&table, "agent.resolve", json!({ "targets": targets })).expect_err("failure")
        };

        let missing = fail(json!(["alpha", "ghost"]));
        assert_eq!(missing.code(), ErrorCode::NotFound);
        assert_eq!(
            failing_position(missing.message()),
            Some(1),
            "{}",
            missing.message()
        );
        assert!(
            missing
                .message()
                .starts_with("targets[1]: no agent called 'ghost'"),
            "{}",
            missing.message()
        );
        assert!(
            missing.message().ends_with(" — candidates: alpha, twin"),
            "{}",
            missing.message()
        );

        let ambiguous = fail(json!(["alpha", "alpha", "twin"]));
        assert_eq!(ambiguous.code(), ErrorCode::Conflict);
        assert_eq!(
            failing_position(ambiguous.message()),
            Some(2),
            "{}",
            ambiguous.message()
        );
        assert!(
            ambiguous
                .message()
                .starts_with("targets[2]: more than one agent is called 'twin'"),
            "{}",
            ambiguous.message()
        );

        // Blank words are checked before reaching the resolver — a later blank word is rejected before a missing name at an earlier
        //   position (0).
        let blank = fail(json!(["ghost", "alpha", "  ", ""]));
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        assert_eq!(
            failing_position(blank.message()),
            Some(2),
            "{}",
            blank.message()
        );
        assert!(
            blank.message().starts_with("targets[2]: blank value"),
            "{}",
            blank.message()
        );
        assert!(
            blank.message().ends_with("(also blank: targets[3])"),
            "the remaining blank positions are named at once too: {}",
            blank.message()
        );
    }

    #[test]
    fn resolve_takes_one_to_eight_targets() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        for count in [0, RESOLVE_TARGETS_MAX + 1] {
            let err = call(
                &table,
                "agent.resolve",
                json!({ "targets": vec!["alpha"; count] }),
            )
            .expect_err("count out of range");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{count}");
            assert_eq!(
                failing_position(err.message()),
                None,
                "a count rejection is not one word's failure, so there is no head: {}",
                err.message()
            );
        }
        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": vec!["alpha"; RESOLVE_TARGETS_MAX] }),
        )
        .expect("up to the cap is accepted");
        assert_eq!(
            out["agents"].as_array().map(Vec::len),
            Some(RESOLVE_TARGETS_MAX)
        );
    }

    /// ★The answer shape is pinned to a file★ — the shell does not depend on this crate and reads the answer as untyped JSON, and the shell tests' fake daemon builds answers from the same file (TRD S28 §6); a changed field name turns either side red.
    // ADR-0307
    #[test]
    fn the_resolve_reply_matches_the_golden_fixture() {
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_resolve_ok.json"))
                .expect("the golden is JSON");
        let host = FakeHost::new();
        for (id, name, live) in [
            ("11111111-1111-4111-8111-111111111111", "alpha", true),
            ("22222222-2222-4222-8222-222222222222", "beta", false),
        ] {
            host.rows.lock().unwrap().push(AgentRosterRow {
                id: id.parse().expect("fixed id"),
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live: live.then_some(AgentStatus::Running),
            });
        }
        let (table, _notify) = wiring(&host);

        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["alpha", "beta"] }),
        )
        .expect("resolves");
        assert_eq!(out, golden);
    }

    #[test]
    fn a_wrong_argument_type_is_invalid_argument() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.rename", json!({ "target": 5 })).expect_err("type mismatch");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn the_list_schema_inlines_the_nested_row_type() {
        let spec = AgentListArgs::SPEC;
        let ok: serde_json::Value =
            serde_json::from_str(spec.ok_schema).expect("the ok schema is JSON");
        assert_eq!(ok["properties"]["agents"]["type"], "array");
        assert_eq!(
            ok["properties"]["agents"]["items"]["properties"]["state"]["type"], "string",
            "a struct declared inside the block is spread inline"
        );
    }

    // ── Waiting-list query and cancel (ADR-0231 · TRD §5-6) ──

    fn listed(id: &str, state: ListedState) -> crate::queued_input::ListedRow {
        crate::queued_input::ListedRow {
            id: id.into(),
            text: format!("text of {id}"),
            state,
        }
    }

    #[test]
    fn the_queued_listing_carries_every_row_state_the_seq_the_incarnation_and_the_halt() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", true, false);
        *host.queued_listing.lock().unwrap() = Some(QueuedListing {
            rows: vec![
                listed("q1", ListedState::Queued),
                listed("q2", ListedState::Unconfirmed),
                listed("q5", ListedState::Sent),
                listed(
                    "q3",
                    ListedState::Cancelling {
                        answer: crate::queued_input::CancelAnswer::NotRemoved,
                        vendor_closed: true,
                    },
                ),
                listed(
                    "q4",
                    ListedState::Cancelling {
                        answer: crate::queued_input::CancelAnswer::Unanswered,
                        vendor_closed: false,
                    },
                ),
            ],
            as_of_seq: Some(7),
            epoch: 4_000_000_000,
            stopped_after_error: true,
        });
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "alpha" }),
        )
        .expect("query succeeds");
        assert_eq!(
            out,
            json!({
                "inputs": [
                    { "id": "q1", "text": "text of q1", "state": "queued", "cancel": null },
                    { "id": "q2", "text": "text of q2", "state": "unconfirmed", "cancel": null },
                    { "id": "q5", "text": "text of q5", "state": "sent", "cancel": null },
                    { "id": "q3", "text": "text of q3", "state": "cancelling",
                      "cancel": { "answer": "not_removed", "vendor_closed": true } },
                    { "id": "q4", "text": "text of q4", "state": "cancelling",
                      "cancel": { "answer": "none", "vendor_closed": false } },
                ],
                "as_of_seq": 7,
                "epoch": 4_000_000_000u32,
                "stopped_after_error": true,
            })
        );
        assert_eq!(*host.queued_calls.lock().unwrap(), vec![(id, None)]);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "does not change the roster"
        );
    }

    #[test]
    fn an_empty_queue_lists_no_rows_and_a_null_seq() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        *host.queued_listing.lock().unwrap() = Some(QueuedListing {
            rows: vec![],
            as_of_seq: None,
            epoch: 0,
            stopped_after_error: false,
        });
        let (table, _notify) = wiring(&host);
        let out = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "alpha" }),
        )
        .expect("query succeeds");
        assert_eq!(
            out,
            json!({ "inputs": [], "as_of_seq": null, "epoch": 0, "stopped_after_error": false })
        );
    }

    #[test]
    fn listing_the_queue_of_an_asleep_unknown_or_ambiguous_agent_is_refused() {
        let host = FakeHost::new();
        host.with_agent("asleep", false, false);
        host.with_agent("twin", true, false);
        host.with_agent("twin", true, false);
        let (table, _notify) = wiring(&host);

        let asleep = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "asleep" }),
        )
        .expect_err("there is no live session");
        assert_eq!(asleep.code(), ErrorCode::NotFound);
        assert!(
            asleep.message().contains("not running"),
            "does not say that an agent in the roster is missing: {}",
            asleep.message()
        );
        for (target, code) in [
            ("nobody", ErrorCode::NotFound),
            ("twin", ErrorCode::Conflict),
        ] {
            let err = call(
                &table,
                "agent.listQueuedInputs",
                json!({ "target": target }),
            )
            .expect_err("pointing fails");
            assert_eq!(err.code(), code, "{target}");
        }
        let blank = call(&table, "agent.listQueuedInputs", json!({ "target": " " }))
            .expect_err("blank value");
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        assert_eq!(
            host.queued_calls.lock().unwrap().len(),
            1,
            "if the pointing does not stand, it does not reach the manager (asked only once, for the asleep agent)"
        );
    }

    /// The 「cancel result」 of the daemon row of TRD §7-1 — the session's answer is translated line by line into the verb response.
    #[test]
    fn cancel_translates_every_session_answer() {
        let answers: Vec<(Result<CancelOutcome, CancelError>, Result<&str, ErrorCode>)> = vec![
            (Ok(CancelOutcome::Cancelled), Ok("cancelled")),
            (Ok(CancelOutcome::Requested), Ok("requested")),
            (Err(CancelError::NotFound), Err(ErrorCode::NotFound)),
            (
                Err(CancelError::Write(PtyError::WriteFailed(
                    "pipe closed".into(),
                ))),
                Err(ErrorCode::Internal),
            ),
        ];
        for (answer, expected) in answers {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.cancel_answer.lock().unwrap() = Some(answer);
            let (table, notify) = wiring(&host);

            let got = call(
                &table,
                "agent.cancelQueuedInput",
                json!({ "target": "alpha", "input_id": "q1" }),
            );
            match expected {
                Ok(word) => assert_eq!(got.expect(&label), json!({ "outcome": word }), "{label}"),
                Err(code) => {
                    let err = got.expect_err(&label);
                    assert_eq!(err.code(), code, "{label}");
                    if code == ErrorCode::Internal {
                        assert!(
                            err.message().contains("stays in the cancelling state"),
                            "a write failure says the entry stays pending cancel: {}",
                            err.message()
                        );
                    }
                }
            }
            assert_eq!(
                *host.queued_calls.lock().unwrap(),
                vec![(id, Some("q1".to_string()))],
                "{label}: the resolved id and input_id are handed over as is"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "{label}: does not change the roster"
            );
        }
    }

    /// ★The failure text calls it by the roster name★ — the daemon's common entry rewrites `target` as the resolved id, so carrying the token as is
    /// would make the caller read a UUID it did not type.
    #[test]
    fn cancel_failures_name_the_agent_by_its_roster_name_even_when_targeted_by_id() {
        for answer in [
            CancelError::NotFound,
            CancelError::Write(PtyError::WriteFailed("pipe closed".into())),
        ] {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.cancel_answer.lock().unwrap() = Some(Err(answer));
            let (table, _notify) = wiring(&host);

            let err = call(
                &table,
                "agent.cancelQueuedInput",
                json!({ "target": id.to_string(), "input_id": "q1" }),
            )
            .expect_err(&label);
            assert!(
                err.message().contains("'alpha'"),
                "{label}: {}",
                err.message()
            );
            assert!(
                !err.message().contains(&id.to_string()),
                "{label}: does not carry the id in the name position: {}",
                err.message()
            );
        }
    }

    #[test]
    fn cancel_requires_an_input_id_and_refuses_a_blank_one_before_touching_the_agent() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        let (table, _notify) = wiring(&host);

        for args in [
            json!({ "target": "alpha" }),
            json!({ "target": "alpha", "input_id": "" }),
            json!({ "target": "alpha", "input_id": "   " }),
            json!({ "target": " ", "input_id": "q1" }),
        ] {
            let err = call(&table, "agent.cancelQueuedInput", args.clone()).expect_err("rejection");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
        }
        let unknown = call(
            &table,
            "agent.cancelQueuedInput",
            json!({ "target": "nobody", "input_id": "q1" }),
        )
        .expect_err("pointing fails");
        assert_eq!(unknown.code(), ErrorCode::NotFound);
        assert!(
            host.queued_calls.lock().unwrap().is_empty(),
            "if the arguments or pointing do not stand, it does not reach the manager"
        );
    }

    /// The link's answer is translated line by line into the verb outcome (TRD S21-chat-ux §3-3).
    // ADR-0237
    #[test]
    fn interrupt_translates_every_transport_answer() {
        let answers: Vec<(Result<(), PtyError>, Result<&str, ErrorCode>)> = vec![
            (Ok(()), Ok("requested")),
            (
                Err(PtyError::NotFound(AgentId::new_v4())),
                Err(ErrorCode::NotFound),
            ),
            (
                Err(PtyError::Unsupported("no turn in flight".into())),
                Err(ErrorCode::Conflict),
            ),
            (
                Err(PtyError::WriteFailed("pipe closed".into())),
                Err(ErrorCode::Internal),
            ),
        ];
        for (answer, expected) in answers {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.interrupt_answer.lock().unwrap() = Some(answer);
            let (table, notify) = wiring(&host);

            let got = call(
                &table,
                "agent.interrupt",
                json!({ "target": id.to_string() }),
            );
            match expected {
                Ok(word) => assert_eq!(got.expect(&label), json!({ "outcome": word }), "{label}"),
                Err(code) => {
                    let err = got.expect_err(&label);
                    assert_eq!(err.code(), code, "{label}");
                    assert!(
                        err.message().contains("'alpha'"),
                        "{label}: calls it by the roster name: {}",
                        err.message()
                    );
                }
            }
            assert_eq!(*host.interrupt_calls.lock().unwrap(), vec![id], "{label}");
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "{label}: does not change the roster"
            );
        }
    }

    #[test]
    fn interrupt_refuses_a_blank_or_unknown_target_before_touching_the_agent() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        let (table, _notify) = wiring(&host);

        let blank =
            call(&table, "agent.interrupt", json!({ "target": " " })).expect_err("blank value");
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        let unknown = call(&table, "agent.interrupt", json!({ "target": "nobody" }))
            .expect_err("pointing fails");
        assert_eq!(unknown.code(), ErrorCode::NotFound);
        assert!(
            host.interrupt_calls.lock().unwrap().is_empty(),
            "if the arguments or pointing do not stand, it does not reach the manager"
        );
    }

    #[test]
    fn the_queue_list_schema_inlines_the_row_and_the_cancel_pair() {
        let ok: serde_json::Value =
            serde_json::from_str(AgentListQueuedInputsArgs::SPEC.ok_schema).expect("ok schema");
        let row = &ok["properties"]["inputs"]["items"]["properties"];
        assert_eq!(row["state"]["type"], "string");
        assert_eq!(
            row["cancel"]["anyOf"][0]["properties"]["vendor_closed"]["type"],
            "boolean"
        );
        let required: Vec<&str> = ok["required"]
            .as_array()
            .expect("required")
            .iter()
            .map(|v| v.as_str().expect("string"))
            .collect();
        assert_eq!(required, vec!["inputs", "epoch", "stopped_after_error"]);
    }

    // ── Usage (`usage.*` — TRD S21 usage-limit-slot §1-6) ───────────────────────────────────────────

    fn usage_table(usage: &Arc<FakeUsage>) -> CommandTable {
        make_table(
            FakeHost::new() as Arc<dyn AgentCommandHost>,
            Arc::new(FakeNotify::default()) as Arc<dyn RosterChanged>,
            Arc::clone(usage) as Arc<dyn UsageCommandHost>,
            crate::library_commands::fake::FakeLibrary::unplugged(),
        )
    }

    fn usage_row() -> UsageVendorRow {
        UsageVendorRow {
            backend: AgentBackend::Claude,
            account_key: "default".to_owned(),
            plan: Some("max".to_owned()),
            windows: vec![UsageWindowRow {
                window: UsageWindowWord::FiveHour,
                label: None,
                used_pct: Some(40.0),
                left_pct: Some(60),
                resets_at: Some(1_900_000_000),
                age_secs: 3,
                expired: false,
            }],
            in_flight: false,
            served: UsageServedWord::Fresh,
            state: UsageStateWord::Failed,
            next_attempt_in_secs: Some(70),
            retry_in_secs: None,
            detail_kind: Some("rpc_error".to_owned()),
            detail_code: Some(-32_603),
            upstream: Some("upstream text".to_owned()),
        }
    }

    /// The two verbs go to their own methods of the same port (with the received word as is), the answer is a flat row without a layer, and the answer shapes of the two verbs
    /// are the same — even though the return struct of `usage.refresh` is built separately, the JSON is one row.
    #[test]
    fn the_usage_verbs_answer_the_ports_row_flat_and_alike() {
        let usage = Arc::new(FakeUsage::default());
        *usage.row.lock().unwrap() = Some(usage_row());
        let table = usage_table(&usage);

        let got = call(&table, "usage.get", json!({ "backend": "Claude" })).expect("success");
        let refreshed =
            call(&table, "usage.refresh", json!({ "backend": "Codex" })).expect("success");

        assert_eq!(
            got,
            serde_json::to_value(usage_row()).expect("row serialization")
        );
        assert_eq!(got["state"], "Failed");
        assert_eq!(got["served"], "Fresh");
        assert_eq!(got["windows"][0]["window"], "FiveHour");
        assert_eq!(got["windows"][0]["left_pct"], 60);
        assert_eq!(got["detail_code"], -32_603);
        assert_eq!(
            refreshed, got,
            "the answers of the two verbs have the same row shape"
        );
        assert_eq!(
            *usage.calls.lock().unwrap(),
            vec![
                ("get", AgentBackend::Claude),
                ("refresh", AgentBackend::Codex)
            ]
        );
    }

    /// A backend without a field fails **with a value** (no panic) — the same code and the same sentence as the same absence of wire ⟳.
    #[test]
    fn a_backend_without_a_usage_slot_fails_by_value_with_not_found() {
        let usage = Arc::new(FakeUsage::default());
        let table = usage_table(&usage);
        for name in ["usage.get", "usage.refresh"] {
            let err = call(&table, name, json!({ "backend": "Codex" })).expect_err("no field");
            assert_eq!(err.code(), ErrorCode::NotFound, "{name}");
            assert!(
                err.message()
                    .contains("this daemon keeps no usage limits for backend 'Codex'"),
                "{name}: {err}"
            );
        }
    }

    /// ★The bus word (capitalized) reaches the same field key as the wire word (lowercase)★ — all declared words. Building the key from the incoming spelling
    /// would have the two entries hit different fields (TRD §3 #43).
    #[test]
    fn every_declared_backend_reaches_the_usage_key_its_wire_word_reaches() {
        let every = |backend: &AgentBackend| match backend {
            // If the declared words grow, compilation stops here — add to the list below.
            AgentBackend::Claude | AgentBackend::Codex => (),
        };
        for backend in [AgentBackend::Claude, AgentBackend::Codex] {
            every(&backend);
            let bus =
                usage_vendor_of(&backend).unwrap_or_else(|| panic!("the prober of {backend:?}"));
            let wire_word = backend_word(&backend).to_ascii_lowercase();
            let wire = crate::backend::usage_probe_for(&wire_word)
                .unwrap_or_else(|| panic!("the prober of {wire_word}"))
                .key();
            assert_eq!(bus, wire, "{backend:?}");
        }
    }

    /// The return schemas of the two verbs are the same down to the bytes — with two return structs, a field could grow on only one side (`From` blocks it at compile
    /// time, but the advertisement is carried by the schema).
    #[test]
    fn the_two_usage_verbs_advertise_one_row_schema() {
        assert_eq!(
            UsageGetArgs::SPEC.ok_schema,
            UsageRefreshArgs::SPEC.ok_schema
        );
        let ok: serde_json::Value =
            serde_json::from_str(UsageGetArgs::SPEC.ok_schema).expect("ok schema");
        let window = &ok["properties"]["windows"]["items"]["properties"];
        assert_eq!(
            window["window"]["enum"],
            json!(["FiveHour", "Weekly", "ModelWeekly"])
        );
        assert_eq!(
            ok["properties"]["state"]["enum"],
            json!([
                "Ready",
                "NotInstalled",
                "NeedsLogin",
                "Unavailable",
                "Failed",
                "Rejected"
            ])
        );
        assert_eq!(
            ok["properties"]["served"]["enum"],
            json!(["Fresh", "Cached"])
        );
        assert_eq!(
            ok["properties"]["detail_code"]["anyOf"][0]["type"], "integer",
            "upstream numbers are carried as JSON numbers"
        );
    }
}
