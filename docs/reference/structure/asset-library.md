# Asset library structure

> **Live reference, edited in place.** The picture and the cross-file contracts of `engram-dashboard-asset-library`; the code holds the details. Anchors are file and symbol names only. The crate's `lib.rs` header is the canonical short form of its boundary and gates.

**Whole picture** = `docs/reference/architecture.md` · **Launch argv per backend** = `agent-backend.md` (this folder)

---

## What it is

The preset library: instruction, skill, settings and preset files under one library root folder, plus three built-ins (`engram:/default` · `engram:/priming` · `engram:/settings/default`) shipped in the install's `builtin\` folder. The crate reads and checks those files, keeps a catalog of them, resolves what an agent actually receives from its attached presets, and makes that agent's launch materials. `LibraryService` is the daemon's real `LaunchKitSource`; the only other implementation is agent `NoopLaunchKits`.

**Boundary** (ADR-0311): its workspace dependencies are exactly agent · platform · command · base. agent defines the ports and does not know this crate. There is no daemon call, no network client, no process runner (ADR-0359), no OS `cfg`, and no test feature. CI gates `asset-library gate 1`–`4` in `.github/workflows/ci.yml` enforce this.

**Backend extensibility** (ADR-0367): backend tags and raw-field names are strings, checked only through `backends::Backends`. Every per-backend choice is a `SettingsCheck` answer, so adding a backend to agent changes nothing here.

## Modules

| Module | Responsibility |
|---|---|
| `lib.rs` | The service: values, catalog reads (list, refresh), resolution entry points |
| `gate.rs` | Library lock, scan tickets, generations, first scan |
| `catalog.rs` | In-memory item table, per-item writes, reference index |
| `store.rs` | Every disk access under the library root (`FsStore`) |
| `path.rs` | Reference spellings (library path · built-in id · folder entry) |
| `format/` | The only YAML: preset/settings readers, front matter, whole-file preset writer |
| `validate.rs` | Settings-asset and skill checks through the backend port |
| `resolve.rs` | Pure resolution: expansion, problem rows, notices |
| `refs.rs` | Who reaches / refers to an item |
| `builtin.rs` | The three built-ins, read once |
| `folder.rs` | Bounded, budgeted IO on folders outside the library |
| `launch.rs` | `LaunchKitSource` impl: check, prepare, release, boot sweep |
| `backends.rs` · `excerpt.rs` | Registry seam · list excerpt |

## Wiring

- **The daemon only wires.** `build_library_service` in daemon `src/lib.rs` is the one builder, shared by production and the `roundtrip_smoke` bin. It plugs in `AgentBackends`, `FsStore`, `BoundedIo` over `StdFolderOps` + `ThreadSpawn`, the shared `ProfileRegistry`, the builtin folder (`data_dir::find_builtin_dir`), the launch root (`DataLayout::launch_dir`) and `LibraryHost::process`.
- **The service goes into the manager as its launch-kit source.** `build_daemon_wiring_with_store` passes it via `AgentManager::with_launch_ports(LaunchPorts { kits, … })`. The test server passes no `LaunchInputs` and keeps the Noop kits.
- **Boot sweep order.** `build_daemon_wiring` runs `launch::sweep_launch_dir` after the instance lock and before the service is plugged in. Plugged in earlier, the sweep could remove a fresh launch's folder.

**Ports and who fills them**

| Port | Defined by | Filled by |
|---|---|---|
| `LaunchKitSource` | agent `launch.rs` | this crate (`LibraryService`) |
| `SettingsCheck` | agent `backend/settings_check.rs` | each agent backend (`backend/<name>/settings.rs`); reached here only via `Backends` |
| `FrontMatterParser` | agent | this crate (`format::YamlFrontMatter`), handed to the port calls |
| `ProfileRegistry` | agent `profile.rs` | agent; this crate only reads it |
| `LibraryStore` · `FolderOps` · `Spawn` · `Backends` | this crate | daemon passes `FsStore` · `StdFolderOps` · `ThreadSpawn` · `AgentBackends`; tests pass fakes |

★**Production has no library root today**★. `build_library_service` passes `root: None`, and nothing calls `LibraryService::change_root`. So no scan runs, `list` answers `library-unavailable`, and any agent with a library preset attached is blocked with ④. An agent whose backend has a `SettingsCheck` and no library preset still gets a real plan: built-ins only, plus `engram:/default` when the profile's `default_preset` is set.

## Main flow

1. **Read**: the store walks the root; `ScanResult::build` reads and parses outside every lock; the gate applies the result per item into the `Catalog`. Readers (`list`, `resolve_*`) pass `first_scan_join` first.
2. **Check**: `check_settings` and `check_skill` ask the backend port; there is no value or schema check (ADR-0359).
3. **Resolve**: `resolve::resolve` expands `engram:/default`, then the attached presets in order, nested presets depth-first. It takes common items, then the agent's backend's tagged items, and yields the materials plus problem rows and notices.
4. **Launch materials**: `launch_check` reads the presets an agent reaches fresh (the overlay), resolves over them, runs the required folder phase and then the advisory phase, and judges the command line. It returns a `LaunchPlan` with a stamp, or `LaunchBlocked`. `prepare_in` re-reads the stamped sources and writes the merged instructions into `<launch_root>\<id>-<epoch>\`. `release` removes that folder.

## Contracts not visible from one file

- **Lock order.** One `RwLock<State>` (catalog + root), then the turn mutex as a leaf, never the reverse. Walking, reading and parsing run outside the lock; the write lock is taken only to apply. Canonical: `gate.rs` header (ADR-0324).
- **Generations.** `Numbers` holds `change` · `root` · `swap_ins`, each raised inside the write lock. A scan whose walk saw `change` move walks again. A late merge applies only if neither `change` nor `root` moved since the caller noted them before its disk read, and it never removes (ADR-0324 · ADR-0366).
- **Tickets.** Tickets are served FIFO. A root change is exclusive from its ticket to its publish. A refresh joins only a covering scan that is queued and not yet walking (ADR-0366).
- **Catalog writes are per item.** There is no whole-catalog swap: per key the later `read_seq` wins, and a removal needs an absence observed after the read. `Catalog::clear` is for a root change only.
- **Links.** Only the configured root may be a link. A path at or below a link inside the library is never read; a reference to it becomes `path-through-link` (ADR-0365).
- **Problem rows are the block.** Resolution problems are agent `launch::BlockReason` rows; the problem list, the tree ⛔ and the launch block are one list. `Resolution::blocked` orders them ① `Missing` → ② `Broken` → ③ `Conflict` → ④ `LibraryUnavailable`.
- **④ stands alone.** A library preset attached while the root is unset or unusable makes the whole answer that one row: no expansion, no folder IO, no command-line check (ADR-0361).
- **Built-ins never block.** An unreadable built-in becomes an empty stand-in and adds one `builtin-unavailable` notice (ADR-0361). The priming reaches an agent only through a real plan's `include_priming`; Noop launches carry none (ADR-0370).
- **One launch, one scope.** The manager opens `begin_launch` once per launch and passes that `LaunchScope` to every `check_in`, `prepare_in` and `working_folder`. The scope holds one `LaunchIo` (one budget, one answer set) and the working folder the last check decided, so the spawn runs where the check looked.
- **The check does not wait on the library.** `launch_check` skips the first-scan entrance. It reads its files fresh and hands the merge off (`hand_off_merge`). `prepare_in` takes no library lock; a stamped source that changed or vanished is `PrepareError::Stale`.
- **Folder IO is bounded.** Every stat, list, canonicalize or read on a folder outside the library goes through `BoundedIo` and never runs under the library lock. A launch budget is 60 s required plus a separate 2 s advisory allowance (agent `LAUNCH_REQUIRED_FOLDER_WAIT_SECS` · `LAUNCH_ADVISORY_FOLDER_WAIT_SECS`). A read outside a launch gets `IO_LIMIT` (ADR-0360).
- **Class B rule.** Required and advisory asks never share a batch or a wait; required runs first and alone. Advisory work runs only when nothing blocks, and a missing advisory answer is never substituted. Canonical: `folder.rs` header.
- **Removal is best effort.** `release` and the boot sweep never follow a link; they detach it. They never wait, retry, lock or panic, and anything they cannot remove stays for the next boot sweep.

## Where to look

- Lock and generation rules: `gate.rs` header · `Gate::late_merge` · `Gate::change_root`
- Catalog write rules: `catalog.rs` header · `Catalog::apply_scan` · `Catalog::apply_items`
- Store rules (links, names, listing sequence): `store.rs` header · `LibraryStore` · `admits`
- Item kind by name: `catalog.rs` `FileKind::of` · `format::sniff_kind`
- File formats and parse codes: `format::parse_preset` · `format::parse_settings` · the code consts in `format/read.rs`
- Writer: `format::rewrite_lists` (ADR-0358)
- Reference rules: `path.rs` header · `LibPath::parse` · `catalog::key` (catalog key) vs `LibPath::key` (disk key) (ADR-0315 · ADR-0362)
- Expansion and problem rows: `resolve.rs` header · `resolve` · `Resolution::blocked`; reason codes are the consts in `resolve.rs`, `lib.rs`, `gate.rs`, `launch.rs` and agent `launch.rs`
- The launch check and prepare: `launch.rs` header · `LibraryService::launch_check` · `LaunchKitSource::prepare_in` · `merge_instructions`
- Folder IO: `folder.rs` header · `BoundedIo` · `LaunchWaits::LAUNCH`
- Built-ins: `builtin.rs` header · `file_of`
- Daemon wiring: daemon `src/lib.rs` `build_library_service` · `build_daemon_wiring`
- Manager side of the launch: agent `launch.rs` `LaunchKitSource` · `manager/prelaunch.rs`

**Not in the crate yet:** the library root config, edits (writers through `LibraryService::change`, the profile-edit port), relink, create, and the `library.*` command declarations.
