# TRD — the asset-library crate: path · format · storage seam · scan · settings check · resolution · reference walk · built-ins · launch materials + gates (S25 step 7)

| Item | Value |
|---|---|
| **Status** | **Draft 2 · 2026-11-11** — the user decisions of 2026-11-11 applied (§10-1 · §10-2): U1 (a) · U2 changed (one 10 s limit, folder IO concurrent) · U3 (a) and U4 (a) by main · U5 (a) · U6 → ADR-0358 (`yaml-edit` dropped; whole-file rewrite with serde-saphyr); the backend-extensibility requirement added (§1-1 · §3-1 · §8-3). Not reviewed. Draft 1 = the 2026-11-10 file (not committed). |
| **Canonical ADRs** | ADR-0311 (the crate, its dependency set, its four gates) · ADR-0324 (locking: one `RwLock`, scan-generation ticket FIFO, conditional late merge) · ADR-0332 (settings check in process) · **ADR-0358** (preset edits rewrite the whole file with serde-saphyr — supersedes ADR-0333, amends ADR-0336 and ADR-0338) · ADR-0334 (launch materials through ports; backend knowledge in `backend`) · ADR-0336 (server component choices, as amended) · ADR-0341 (folder entries) · ADR-0353 · ADR-0355 · ADR-0357 (step-6 duties of `check`) · ADR-0315 (references) · ADR-0325 (path key · file identity) · ADR-0327 (priming source = `builtin\`) · ADR-0328 (one merged claude `--settings`) · ADR-0329 (backend tags on preset items) · ADR-0331 (no warning category) · ADR-0335 (atomic write · no auto-cleanup of temp files) · ADR-0337 (codex materials) · ADR-0338 (draft + save — decision 6 writer amended). |
| **Line pointers** | Code anchors re-read at `60002c49`; HEAD is now `77859e6b`, which changes only `docs/decisions/` (ADR-0358 and its amendment notes — `git diff --stat 60002c49 77859e6b`), so every code pointer still holds. Agent paths are under `crates/engram-dashboard-agent/src/` unless stated otherwise. |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 7 (L893), "ADR-0341 in 7" (L895), "Handed over by step 5" (L897), "Handed over by step 6" (L899); §1-2 (L62–L139), §1-3 (L140–L159), §1-4 (L161–L235), §1-5 (L237–L331), §1-12 (L560–L595), §4 (L773–L804), §6-2 (L984–L1019). |

## 1. Scope and relation to the S25 TRD

**What this step builds.** A new workspace crate `crates/engram-dashboard-asset-library/` (Rust `engram_dashboard_asset_library`) with the parts the parent assigns to step 7, plus the small additions outside it that those parts need:

- **inside the new crate** — `path` (`LibPath` · `AssetRef` · `BuiltinId` · preset items) · `format` (YAML reading with serde-saphyr; the whole-file preset writer on serde-saphyr's serializer over an order-keeping document tree — ADR-0358; the real `FrontMatterParser`) · the storage seam `LibraryStore` (`FsStore` · `MemStore`) and the scan · `catalog` (item table, content tags, racy rule, preset-side reverse index) · `validate` (settings check over the backend settings-check port; the schema set; the fetch and CLI-version seams) · the service `LibraryService` with the ADR-0324 locking (read lock · scan-generation ticket FIFO · first-scan join · refresh merging · conditional late merge) and the read entrances `list` · `refresh` · `resolve` (pure — expansion, tags, dedup, cycle, conflicts, problem list, notices, folder entries through an injected stat seam) · `refs` (the reusable reference walk — G37) · `builtin` (`engram:/…`) · `folder` (bounded, concurrent folder IO) · `launch` (`impl LaunchKitSource for LibraryService` — `check` · `prepare` · `release` — and the boot sweep of `launch\`);
- **outside it** — base `file::is_temp_name` (M65) · a folder-path normalizer in platform `fs` (§1-2) · three additive methods on the agent settings-check port (the merge-origin map handed over by step 5; the working-folder rule handed over by step 6 (d); the backend's notice codes, needed for backend extensibility — §1-1) · one additive `LaunchPlan` field (B7) · the notice wording for the new codes in `launch.rs` (U3 · U4) · the four CI gates and the `/qa` binding row (M17).

### 1-1. Design rule — backend extensibility (user 2026-11-11)

User: 「나중에 백엔드 계속 추가될텐데 최대한 공용화로 하고(예를들어 ENUM 추가하면 모두 자동으로 되게 한다던가.)」. **Adding a backend to agent (a new `AgentCommand` variant, its `backend_word`, its `SettingsCheck` in `settings_checks()`) must make tags, settings raw fields, the selection of what an agent loads, the schema check and the notices work with no asset-library change.** Concretely:

- A backend tag and a settings raw-field name are **strings**; asset-library accepts a word only if `settings_check_for(word)` (`backend/mod.rs:776`) knows it, and compares case-insensitively through that function. No `enum Backend` in asset-library.
- Everything that differs per backend or mode comes from a port answer: `loads` · `accepts_skill_name` · `merge` / `merge_origins` · `unknown_keys` · `raw_to_json` · `command_line_args` · `command_line_fits` · `skill_format` · `folder_entry_blocks` · `working_folder_blocks` · `folder_load_set` · `native_skill_roots` · `root_skills` · `schema_source` · `version_probe` · and the notice codes (`notice_codes`, §3-9). asset-library branches only on those answers, never on the backend word.
- **No `claude` / `codex` literal in asset-library outside tests and the built-in content files** (reviewed in §8-3).

Draft 1 broke this in two places (§10-2 B20): asset-library would have chosen the three codex-named notice codes (`CODEX_TERMINAL_OMITTED` · `CODEX_TERMINAL_FOLDER_OMITTED` · `CODEX_SAME_NAME_SKILLS`, `launch.rs:350-360`) by knowing that the backend is codex, and the draft named the working-folder refusal code as if asset-library owned it. Both now come from the port. Draft 1's prose that says "codex terminal" / "claude merged" describes what today's port answers produce, not branches in the code.

**Nobody calls the crate yet.** No daemon edge, no command, no manager wiring. Every launch stays Noop until step 8. Build and the full regression stay green because nothing outside the crate's own tests reaches the new code, and the agent additions are additive (existing callers and goldens unchanged).

**What it leaves to later steps:**

| Step | What stays there (not built here) |
|---|---|
| 18 | CLAUDE.md 「백엔드 모듈 맵」 · 「빌드·검증 명령」 · 「의존성」 lines (after user confirmation — M1). This step only gathers the text (§12). |
| 17 | The in-repo `builtin\` sources (「기본」 preset · priming `.md` · settings `.yaml`) and one shipped schema set per backend in the release zip; the dev · test · `ENGRAM_DATA_DIR` lookup rule (§2-4 1). Here `builtin\` and the shipped schema folder are constructor values; tests use fixtures. |
| 8 | Daemon wiring: `LibraryService` → manager as `LaunchKitSource`, `DataLayout` values, `daemon_id`, the boot call of the launch sweep **before** the port is plugged (§5-2 4 (c2) ⑥), the first-turn hash recording (step-6 hand-over to 8 (a)), removal of the debug launch-block hook (step-20 hand-over), the real CLI lane. |
| 9a | `config.rs` (`library.json` · `setRoot` · first-start default root · moving root generation), `edit.rs` (`preset.add/remove/reorder/setBackend/save/check` · `rev` with sha2), `relink.rs`, the profile-edit port cell, `newFolder` · `newFile`, the four block reasons through edits. Step 7 provides the writer (`format::write`) and the walk (`refs`) they use. |
| 10a | `LibraryCommandHost` (agent) and `impl LibraryCommandHost for LibraryService`; command declarations; `engram help library`. Step 7's inherent methods (`list`, `refresh`, `resolve_preset`, …) are what those bodies call. |
| 9b · 10b | `ops.rs` (move · rename · copy · delete with reference fixing), the `.trash` bundle naming in use, `.git/info/exclude`, restore, emptying (platform OS trash). Step 7 defines and implements the storage-seam methods those use (`rename` · `copy` · `trash` — B3). |
| 19 | The real fetch port (ureq) and the real CLI-version runner; the start-of-daemon schema refresh thread; the async-import gate (§2-4 7). Step 7 defines both seams and the refresh method, tested with fakes. |
| 11 · 13 | Notices in the webview; the folder-line wording (§2-4 8); the preset edit area's per-backend rows shown only when added (§12 — user 2026-11-11). |

## 2. Premises checked against code

Re-read at `60002c49` (still valid at `77859e6b` — docs-only since). **Bold verdict = the premise does not hold as the parent or a hand-over states it.**

| # | Premise (source) | Verdict | Evidence |
|---|---|---|---|
| 1 | Agent defines `LaunchKitSource { check, prepare, release }`; `check(&AgentProfile)`; `PrepareError { Stale, Failed }` (§1-5 L267) | True | `launch.rs:553-590` (trait) · `:525-532` (`PrepareError`) · `NoopLaunchKits` `:591-627`. `check` is documented read-only, may read the registry and `workspace_lock`, and sees its own reservation as `Preparing` (`:556-567`). A profile not yet in the registry also arrives here (pre-insert check — `manager/prelaunch.rs:28-29`). |
| 2 | `LaunchPlan` carries the folder list and the extra skill roots (ADR-0341 in 16) | True | `launch.rs:154-180` (`folders` `:174` · `extra_skill_roots` `:179`). **No field carries what `check` read** (tags of the sources) — `prepare` can detect a vanished source but not a changed one without one (§5-2 4 (c) "vanished **or changed** ⇒ `Stale`"). B7. |
| 3 | The settings-check port has a merge but no per-asset attribution (step-5 hand-over) | True | `backend/settings_check.rs:49` `fn merge(&self, layers) -> Option<Value>` — no origin. claude `Some` (`backend/claude/settings.rs:76-78`), codex `None` (`backend/codex/settings.rs:133-135`). An additive method is needed (§3-9). |
| 4 | `FrontMatterParser` has `paths` · `subagent` · `skill` · `is_yaml_value`; only fakes implement it (step-6 hand-over (c)) | True | `settings_check.rs:229-238`. `grep "impl FrontMatterParser"` = test fakes only. The real one is step 7's. |
| 5 | The codex front-matter serde type lives in agent (M72) | True | `settings_check.rs:243-259` `SkillFrontMatter` / `SkillMetadata` (`Deserialize`, unknown keys ignored). |
| 6 | One registry of backend checks | True | `backend/mod.rs:770-781` `settings_checks()` (two slots today) · `settings_check_for(word)` (ASCII case-insensitive). asset-library gets every backend word from here (§1-1). |
| 7 | `check` can judge the codex working-folder rule on the canonicalized folder (step-6 hand-over (d)) | **False today** | The rule is private: `backend/codex/launch.rs:162` `fn cwd_reaches_codex_unchanged`. `command_line_fits` does not apply it — `backend/codex/cmdline.rs:129-140` counts `--cd <cwd>` length only; the `CommandLine.cwd` doc (`settings_check.rs:158`) mentions the line, not the characters. An additive port method is needed (§3-9 · B6). |
| 8 | The skill-link layout is not visible outside agent (step-6 hand-over (f)) | True | `launch.rs:911-913` `SKILL_LINKS_PARENT` / `SKILL_LINKS_DIR` are `pub(crate)`; the doc (`:905-910`) says a release outside the crate must detach every directory link at any depth. |
| 9 | A public link test exists to reuse | True | `StdLaunchFs::is_link` (`launch.rs:674-700`, `symlink_metadata().file_type().is_symlink()` — name-surrogate reparse points incl. junctions); platform `fs::unlink_dir` (`platform/src/fs.rs:19`). |
| 10 | The manager canonicalizes `cwd` with `dunce` before `materialize` | True | `manager.rs:1566` `dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone())`. `check` judges the same value (hand-over (d)). |
| 11 | Notice codes and wording | True — **but the codes are backend-named constants in agent** | `launch.rs:350` `CODEX_TERMINAL_OMITTED` · `:354` `CODEX_TERMINAL_FOLDER_OMITTED` · `:360` `CODEX_SAME_NAME_SKILLS`; `notice_text` `:373` with `own_wording` `:394`; any other code gets the generic 「알림(<code>): 의도된 동작입니다.」 (ADR-0357 decision 3). For asset-library to emit them without knowing the backend, the port must hand them out (§3-9 `notice_codes` · B20). |
| 12 | Notice `args` carry identity only | True | `notice_fingerprint` `launch.rs:485` keys on `code` + canonical `args` (no `line`). A diagnostic in `args` would re-show a notice. |
| 13 | Profile-edit port shape | True | `profile_edit.rs:52-88` (`set_attachments` · `rewrite_preset_paths` · `remove_preset_paths` · `workspace_lock`). Step 7 calls none of them (`check` needs none; edits are 9a). |
| 14 | Profile fields | True | `profile.rs:126-128` `AttachedPreset { path: String }` · `:167` `default_preset` · `:171` `presets` · `:181` `last_applied_instructions_hash` · `:193` `notice_fingerprint`; `command` (`:155`) carries the variant and `output_format` (`AgentOutputFormat` — `Terminal` · `StreamJson`). |
| 15 | Agent-side references are read straight from the registry (M5) | True | `profile.rs:1075` `ProfileRegistry::attachments_of(id)` · `:1080` `attachments()` (pub). Writes are `pub(crate)` (`:1068` doc). |
| 16 | `base::file::is_temp_name` exists (M65 · brief) | **False** | Not in `base/src/file.rs`. Only private `temp_owner(file_name, target)` (`:515`) and `TEMP_MARK = ".tmp"` (`:498`); `temp_path` writes `<name>.tmp<pid>.<n>` (`:500-507`). Step 7 adds the public name-only test (§3-10). |
| 17 | base `write_atomic` creates a missing parent | True | `base/src/file.rs:338-348` (doc: "creates the missing folders above it"). So `FsStore::write_atomic` checks the parent first (M65). |
| 18 | platform has `link_dir` · `unlink_dir` · `path_key` · `same_path` · `file_identity`; a folder-path normalizer exists | Partly — **no normalizer** | `platform/src/fs.rs:18-20` re-exports the five. `fs/path_key.rs:13-20` states the caller must pick the spelling (separators, `.`/`..`, trailing separator, `\\?\`). No normalizer anywhere (`base::path::normalize_spelling` `base/src/path.rs:14` only strips quotes and turns `\` into `/`). Step 7 adds it (§3-10). |
| 19 | The cmd guards | True | `platform/src/shell.rs:61` `path_is_cli_safe` · `:75` `text_is_cli_safe`. claude's `folder_entry_blocks` calls the first (`backend/claude/settings.rs:100-102`). |
| 20 | `file_identity` answers `Unsupported` for a FAT root | True (doc) | `platform/src/fs/identity.rs:20-27`. The scan takes it as "no identity", not a failure (§6-2 L1015). |
| 21 | Workspace dependency style | True | Root `Cargo.toml` has no `[workspace.dependencies]`; each crate states its versions. Members list = root `Cargo.toml` `members` (10 entries — this step adds the 11th). |
| 22 | Which packages are new | True | `Cargo.lock` already has `walkdir 2.5.0` · `sha2 0.10.9` · `junction 2.1.0` · `tempfile 3.27.0` · `dunce 1.0.5`, and of serde-saphyr's `serialize` feature (`base64` · `num-traits` · `zmij` · `nohash-hasher` — serde-saphyr 2.0.0 `Cargo.toml` `[features] serialize`) `base64 0.22.1` · `num-traits 0.2.19` · `zmij 1.0.21`. **New**: `serde-saphyr` (+ its `deserialize` deps `granit-parser` · `annotate-snippets` · `encoding_rs_io`) · `nohash-hasher` · `jsonschema` · `relative-path` · `pulldown-cmark`. `yaml-edit` and `rowan` · `countme` · `text-size` are no longer added (ADR-0358). |
| 23 | The chosen versions are current (§1-12 · ADR-0336) | **Newer releases exist** | `cargo search` 2026-11-10: `serde-saphyr 2.1.1` (chosen 2.0.0 — a caret `"2.0.0"` resolves to 2.1.1) · `jsonschema 0.61.1` (chosen 0.60.2 — a caret stays on 0.60.x) · `relative-path 2.0.3` (2.0.2) · `sha2 0.11.1` (lock 0.10.9) · `pulldown-cmark 0.13.6` · `walkdir 2.5.0` — unchanged. B10. |
| 24 | MSRV fits the toolchain | True for the measured versions | `rust-toolchain.toml` = 1.95.0. Registry copy `serde-saphyr 2.0.0` `rust-version = "1.89"`; `walkdir 2.5.0` none. 2.1.1 / 0.61.1: not determined (not in the local registry). |
| 25 | New packages pass the license gate (`ci.yml:1628-1630` → `scripts/gen-third-party-licenses.mjs`, which fails on a crate with no license text unless listed in `scripts/licenses/synthesized-crates.txt`) | True as far as checked | With `yaml-edit` dropped (ADR-0358 — its package had no license file), no new entry is expected. Checked locally: `serde-saphyr 2.0.0` ships `LICENSE-APACHE` · `LICENSE-MIT`; `nohash-hasher 0.2.0` (`Apache-2.0 OR MIT`) ships both; `zmij` (`MIT`) is already in the lock. `granit-parser` · `annotate-snippets` · `encoding_rs_io` · `jsonschema` · `relative-path` · `pulldown-cmark` license files: not determined (not in the local registry) — the gate is run locally in P1/P3. |
| 26 | Gate templates | True | `ci.yml:557-567` messaging regex `engram_dashboard_(agent|base|daemon|protocol|command|platform|transport)` (no `asset_library`) · `ci.yml:1207-1235` the cli name-set gate (template for gate ②) · `ci.yml:539-555` the agent `use tauri` gate (template for ④) · per-crate test steps `ci.yml:286-301` (template for ①). `ci.yml:176-185` `cargo test --workspace` already runs the new crate. The local gates job runs the CI steps from the YAML (`.claude/skill-bindings/qa-verify.md` 「gates 잡」 `:47-75`). |
| 27 | `LibraryCommandHost` exists | False (expected) | `grep LibraryCommandHost` = 0 hits. It is 10a's. |
| 28 | `SpawnByCwd` runs the spawn on a blocking thread (§1-4 `lib.rs` row) | True (pointer moved) | `daemon/src/connection_core.rs:1744` `tokio::task::spawn_blocking` around the create body (the parent's `:1596` is stale after step 20). Step 8 re-checks when it plugs the service. |
| 29 | Reason codes are kebab-case | True | `backend/codex/cmdline.rs` `command-line-too-long` · `codex-priming-unsafe-char`; `backend/codex/settings.rs` `codex-config-line-invalid` · `codex-skill-front-matter` … The parent's placeholder `folder_not_found` (§1-3 L159) is snake-case → B8 uses `folder-not-found`. |
| 30 | codex unknown keys are ⚠ but `Resolution` has no warning field | Resolved by U4 | `backend/codex/settings.rs:138-140` `UnknownKeys::Warn`; ADR-0331 「영향」 flagged it. U4 (a) — a notice (§10-1). |
| 31 | The codex terminal translation refuses a real plan with instruction sources (step-6 hand-over (b)) | True | `backend/codex/launch.rs:115-120`. Generic rule in `check`: when `loads(mode).instructions == BuiltinPrimingOnly`, the plan carries no instruction sources. |
| 32 | `LaunchArtifacts` has no notice channel | True | `launch.rs:917-955`. All notices come from `check`. |
| 33 | The claude preview divergences (step-5 hand-over) are asset-library's | **False** | They are in agent `backend/claude/settings.rs` `folder_load_set` (doc `:104-110`). B11. |
| 34 | `DataLayout` has the folders the service needs | Not checked | `daemon/src/data_dir.rs:207` `struct DataLayout`. Step 8 / 9a. Step 7 takes plain paths. |
| 35 | jsonschema 0.60.2 compiles and passes the SchemaStore claude schema (draft-07 + `$defs`) (§6-2 L1018) | Not determined | Only `jsonschema-0.59.0` is in the local registry; no schema copy in the repository. Pinned by a P5 test with a fixture fetched at implementation time. |
| 36 | The codex release-asset schema path `rust-v<ver>/config-schema.json` | Not determined | §6-2 L1018 "unconfirmed". P5 confirms before writing the fixture; until a copy exists, those settings get syntax + the port's `-c` rules and the "schema not checked" notice. |
| 37 | serde-saphyr can write a value tree back to YAML (ADR-0358) | True | `serde-saphyr-2.0.0/src/ser/api.rs` `to_string` · `to_string_with_options` (feature `serialize`); `SerializerOptions` (`src/ser/options.rs:70-209`): `indent_step` (2) · `compact_list_indent` · `empty_as_braces` · `yaml_12` (default `false` = YAML 1.1-safe quoting: `yes`/`on`/`no`/`off` strings are quoted — `src/ser/quoting.rs` `is_ambiguous`). **No line-ending or BOM option** (grep 0), and **serde-saphyr has no value/document type of its own** (grep `pub enum Value` 0). |
| 38 | `serde_json::Value` can be the document tree | **False** | `serde_json 1.0.150` in the lock has no `indexmap` dependency → `preserve_order` is off → `Map` is a `BTreeMap`: a rewrite would sort keys, and a non-string key cannot be held. So the writer uses its own order-keeping tree (§3-4). |

## 3. Module boundaries and signatures

### 3-1. Crate manifest and the dependency rule

```toml
[package]
name = "engram-dashboard-asset-library"   # Rust: engram_dashboard_asset_library

[dependencies]
engram-dashboard-agent    = { path = "../engram-dashboard-agent" }      # ports, profile, backend registry
engram-dashboard-platform = { path = "../engram-dashboard-platform" }   # path_key · same_path · file_identity · link · normalizer · retry_busy · sync_dir
engram-dashboard-command  = { path = "../engram-dashboard-command" }    # (10a) — present now so gate ② is the final set
engram-dashboard-base     = { path = "../engram-dashboard-base" }       # write_atomic · is_temp_name · sync::lock
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde-saphyr = { version = "<B10>", default-features = false, features = ["deserialize", "serialize"] }   # ADR-0358
jsonschema = { version = "0.60.2", default-features = false, features = ["resolve-file", "idna"] }
relative-path = "2.0"
walkdir = "2.5"
sha2 = "0.10"                                                            # 0 new packages (lock 0.10.9)
pulldown-cmark = { version = "0.13.6", default-features = false }
thiserror = "1.0"
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
engram-dashboard-base = { path = "../engram-dashboard-base", features = ["test-support"] }
```

- **Exactly agent · platform · command · base** among workspace crates (ADR-0311 decision 2). `command` has no use in step 7 itself; keeping it now makes gate ② the final name set from day one (B1).
- **No test feature** (ADR-0311 decision 4). `MemStore` and the fakes are `#[cfg(test)]` inside the crate.
- **No network client, no process runner** — the fetch and the CLI-version read are seams (§3-6).
- **Backend extensibility** (§1-1) — no backend word, no backend enum; every per-backend answer comes from `settings_check_for(word)` and the port.
- **No OS `cfg`** in the crate — OS behaviour comes from platform (platform gate 4 list unchanged).

### 3-2. Module map

| File | Holds | Unit |
|---|---|---|
| `lib.rs` | Crate header (invariants, gates, §1-1 rule), `LibraryService`, `LibraryValues`, re-exports | P1 (header) · P6 |
| `path.rs` | `LibPath` · `AssetRef` · `BuiltinId` · `PresetItem` · `FolderRef` · `PathError` | P1 |
| `format/mod.rs` · `format/read.rs` | Kind sniff, `PresetDoc` · `SettingsDoc` · `parse_preset` · `parse_settings` · `ParseError` | P3 |
| `format/frontmatter.rs` | `split_front_matter` (codex `extract_frontmatter` rule) · `YamlFrontMatter: FrontMatterParser` · library skill naming (M13) | P3 |
| `format/tree.rs` · `format/write.rs` | The order-keeping YAML tree `Node`; `rewrite_lists` — whole-file rewrite (ADR-0358) | P3 |
| `store.rs` | `LibraryStore` · `FsStore` · `MemStore` (test) · `ScanListing` · `ContentTag` | P4 |
| `catalog.rs` · `excerpt.rs` | `Catalog` · `Item` · racy rule · reverse index · instruction excerpt | P4 |
| `validate.rs` | `SchemaSet` · settings check · merged-value attribution · skill-format wrapper · `SchemaFetch` · `CliVersionRead` · `refresh_schemas` | P5 (+ P6 for the re-check as a change) |
| `gate.rs` | The scan-generation `Mutex` + `Condvar`: tickets, change count, generations, swap-in count, first-scan state, refresh merging | P6 |
| `builtin.rs` | `Builtins` — `engram:/…` → files under the `builtin\` value | P7 |
| `resolve.rs` | `Target` · `Lookup` · `Resolution` · `resolve` · folder-status attach | P7 |
| `refs.rs` | Reachable closure · reverse references (preset side from `Catalog`, agent side from the registry) · counts | P8 |
| `folder.rs` | `BoundedIo` (stat · list · canonicalize with one time limit, concurrent across paths, one in-flight call per key) · `FolderStatus` · `FolderListing` | P8 |
| `launch.rs` | `impl LaunchKitSource for LibraryService` · volatile layout · `sweep_launch_dir` | P9 |

Names are placeholders where the parent left them so; B1 lists the ones main fixes.

### 3-3. `path.rs`

```rust
pub struct LibPath(RelativePathBuf);          // UTF-8 · '/' only · no leading/trailing '/' · no empty, '.', '..' · no '\' ':' control
impl LibPath {
    pub fn parse(text: &str) -> Result<Self, PathError>;
    pub fn as_str(&self) -> &str;
    pub fn to_path(&self, root: &Path) -> PathBuf;           // disk path under the root
    pub fn from_disk(root: &Path, path: &Path) -> Result<Self, PathError>;   // scan → reference spelling
    pub fn parent(&self) -> Option<LibPath>;
    pub fn file_name(&self) -> &str;
    pub fn key(&self, root: &Path) -> PathKey;               // platform path_key of to_path(root)
    pub fn replace_prefix(&self, from: &LibPath, to: &LibPath) -> Option<LibPath>;   // 9b uses it
}
pub enum BuiltinId { DefaultPreset, Priming, DefaultSettings }   // engram:/default · engram:/priming · engram:/settings/default
pub enum AssetRef { Library(LibPath), BuiltIn(BuiltinId) }
impl AssetRef { pub fn parse(text: &str) -> Result<Self, PathError>; pub fn spelling(&self) -> String; }
pub struct PresetItem { pub target: AssetRef, pub backend: Option<String /* a word settings_check_for knows */>, pub line: u32 }
pub struct FolderRef { pub raw: String, pub normalized: Result<PathBuf, FolderPathError>, pub line: u32 }
```

`AssetRef::parse` decides by prefix: `engram:/` → built-in (an unknown id = `PathError::UnknownBuiltin`); anything else → `LibPath`. `LibPath` refuses `:`, so the two never mix (§1-2). A tag value that `settings_check_for` does not know is a broken line (② — `backend-tag-unknown`); a new backend's word is accepted the moment agent registers it (§1-1).

### 3-4. `format`

**Read** (`read.rs`):

```rust
pub enum Sniff { Preset, Settings, Unknown(String /* the kind value */), Absent }
pub fn sniff_kind(text: &str) -> Sniff;        // lenient: a top-level `kind:` line, even in broken YAML (M14)

pub struct PresetDoc {
    pub version: u32,
    pub description: Option<String>,
    pub presets: Vec<PresetItem>,       // backend always None (a tag here = broken line)
    pub instructions: Vec<PresetItem>,
    pub skills: Vec<PresetItem>,
    pub settings: Vec<PresetItem>,      // backend always None
    pub folders: Vec<FolderRef>,
    pub line_problems: Vec<Reason>,     // per-line breakage that does not stop the read (unknown tag, relative folder …)
}
pub struct SettingsDoc { pub version: u32, pub description: Option<String>, pub raw: Vec<RawField> }   // raw = every other top-level key, file order
pub struct RawField { pub name: String, pub value: serde_json::Value, pub line: u32 }

pub fn parse_preset(text: &str) -> Result<PresetDoc, ParseError>;
pub fn parse_settings(text: &str) -> Result<SettingsDoc, ParseError>;
pub struct ParseError { pub code: &'static str, pub message: String, pub line: Option<u32> }   // → agent `Reason`
```

- serde-saphyr `Options { strict_booleans: true, .. }` everywhere (G49). Item shapes: a plain string or `{path, backend}` (M20) — read through an intermediate value enum so a wrong shape is a line problem with its line, not a whole-file failure (B5: `untagged` loses `Spanned` positions — §6-2 L1018).
- Raw fields are kept by name and value only; which field is "this backend's" is decided later by `settings_check_for(name)` for the agent's word, case-insensitively. A field whose name no backend knows is kept and ignored (a later backend may claim it — §1-1).
- **`folders`**: each item must be a string; `normalized` = `platform::fs::normalize_folder` (§3-10). A relative path or a `LibPath` = `FolderPathError` → broken line ② (§1-2). Missing key = empty list.

**Front matter** (`frontmatter.rs`):

```rust
pub fn split_front_matter(text: &str) -> Option<FrontMatter<'_>>;   // codex extract_frontmatter: lines(); first line trim()=="---"; up to next "---"; BOM not stripped
pub struct FrontMatter<'a> { pub yaml: &'a str, pub first_line: u32, pub body: &'a str }
pub struct YamlFrontMatter;                                          // serde-saphyr, strict_booleans
impl FrontMatterParser for YamlFrontMatter { fn paths(..); fn subagent(..); fn skill(..); fn is_yaml_value(..); }
pub fn library_skill_name(skill_md: Option<&str>, folder_name: &str) -> String;   // front-matter name trimmed+folded, else folder name (M13)
pub fn conflict_key(name: &str) -> String;                                         // lowercase, [^a-z0-9_-] → '-' (M13 · M25)
```

`YamlFrontMatter` is the only real implementation of the agent trait; `validate` passes it to `skill_format` and `folder` passes it to `folder_load_set` / `root_skills` (M63).

**Write** (`tree.rs` · `write.rs` — ADR-0358) — the primitive 9a's `preset.*` and `preset.save` call:

```rust
pub enum Node { Null, Bool(bool), Int(i128), Float(f64), Str(String), Seq(Vec<Node>), Map(Vec<(Node, Node)>) }
impl<'de> Deserialize<'de> for Node;   // a visitor that keeps map order and non-string keys
impl Serialize for Node;               // maps in stored order

pub struct PresetLists { pub presets, pub instructions, pub skills, pub settings: Vec<PresetItem>, pub folders: Vec<String> }
pub fn rewrite_lists(text: &str, lists: &PresetLists) -> Result<String, WriteError>;
pub enum WriteError { Parse(ParseError), NotAPreset, MultipleDocuments, RoundTrip { path: String /* first differing node */ } }
```

`rewrite_lists`:
1. Strip a leading U+FEFF and remember it; remember the line ending (CRLF if the first line break is `\r\n`, else LF; mixed endings are written as the first one — the file is rewritten anyway).
2. Parse the text into `Node` (serde-saphyr, `strict_booleans`). More than one document ⇒ `MultipleDocuments` (a whole-file rewrite would drop the others — refuse, write nothing). Not a top-level map with `kind: preset` ⇒ `NotAPreset`.
3. Replace only the five list nodes (`presets` · `instructions` · `skills` · `settings` · `folders`) in place — an absent key is appended after the last present one; an emptied list is written as `[]` (`empty_as_braces` default). **Every other key, known or not, stays as the parsed node in its original order** (ADR-0358 「영향」: keys Engram does not model survive). An item with a tag is written as a two-key map `{path, backend}` in that order; without one, as a string.
4. Serialize with `to_string_with_options(&node, ser_options! { indent_step: 2 })` — `yaml_12` stays `false`, so strings that a YAML 1.1 reader would take as booleans or numbers are quoted.
5. **Round-trip check** — re-parse the output into `Node` and compare with the tree from step 3; any difference ⇒ `RoundTrip` (code `preset-write-mismatch`), nothing written.
6. Re-apply the line ending (LF → CRLF if remembered) and the BOM.

**Why the round-trip check stays** (does any pre-write check still earn its place?): ADR-0333's gate existed to catch `yaml-edit`'s broken outputs and is gone with it. A different risk remains: a serializer that writes a scalar a reader takes back as another value (a numeric-looking string, a large integer, a float's text, an ambiguous plain scalar) would silently change a user's file — the one failure ADR-0358 does not accept ("must not lose data other than formatting"). The check costs one parse of a small file, so it stays as a data-loss guard, not a shape guard. It is expected never to fire; if it does, the edit is refused with the differing path in `args`.

What is lost on a rewrite, by decision (ADR-0358): comments, blank lines, quoting style, flow vs block style, key spacing, anchors/aliases (an alias is written out as its expanded value — the values stay equal), merge keys (`<<` expanded — values equal), explicit tags (`!!str` etc. — the tagged value stays). Indentation becomes 2 spaces.

### 3-5. `store` and `catalog`

```rust
pub trait LibraryStore: Send + Sync {
    fn scan(&self, root: &Path, under: Option<&Path>, seq: &dyn Fn() -> u64) -> io::Result<ScanListing>;
    fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>>;
    fn tag(&self, path: &Path) -> io::Result<ContentTag>;
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;   // parent missing → NotFound, nothing created
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;             // base copy_atomic
    fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf>;   // rename into <root>\.trash\<bundle>\<same rel path>
    fn exists(&self, path: &Path) -> bool;
}
pub struct ScanListing { pub files: Vec<ScanEntry>, pub folders: Vec<PathBuf>, pub skipped_links: Vec<PathBuf>, pub errors: Vec<(PathBuf, String)>, pub listed_seq: u64 }
pub struct ScanEntry { pub rel: PathBuf, pub tag: ContentTag }
#[derive(PartialEq, Eq)] pub struct ContentTag { pub modified: SystemTime, pub size: u64, pub identity: Option<FileIdentity> }
```

`FsStore` = walkdir + std + base `write_atomic` / `copy_atomic` with `OsHooks { retry: platform retry_busy, sync_dir: platform sync_dir }`. **Link policy (U1 (a))**: `follow_root_links(true)` (the root, or the `under` folder of a partial scan, may itself be a link) and `follow_links(false)`; an entry inside the walk that is a link (file or directory — `StdLaunchFs::is_link` semantics) is not entered and not listed but recorded in `skipped_links`, which `refresh` returns. `filter_entry` drops names starting with `.` and `base::file::is_temp_name` names. `trash` is a same-root rename (B8 · G31); the bundle id is 9b's (§6-5 1).

```rust
pub struct Catalog { items: HashMap<PathKey, Item>, folders: HashMap<PathKey, LibPath>, used_by: HashMap<PathKey, BTreeSet<PathKey>>, racy: HashSet<PathKey>, scanned_at: SystemTime }
pub struct Item { pub path: LibPath, pub kind: ItemKind, pub tag: ContentTag, pub read_seq: u64, pub content: ItemContent, pub schema_gen: u64 }
pub enum ItemKind { Instruction, Skill, SkillFile, Preset, Settings, Broken { claimed: Option<Sniff> } }
impl Catalog {
    pub fn build(listing: ScanListing, range: Option<&LibPath>, prev: Option<&Catalog>, read: &dyn Fn(&Path) -> io::Result<Vec<u8>>, v: &Validator, now: SystemTime) -> Catalog;
    pub fn get(&self, key: &PathKey) -> Option<&Item>;
    pub fn children(&self, folder: Option<&LibPath>) -> Vec<&Item>;
    pub fn merge_item(&mut self, item: Item, may_insert: bool);   // the late merge's write (the rule lives in the service)
}
```

- Kind decision = §1-2 table. A file is re-read only when its tag differs (equality, not "newer") or it carries the racy mark (mtime ≥ scan time − 2 s). Unsupported identity = `None`, not a failure (premise 20).
- `used_by` = preset → items, direct + indirect (Δ3). Agent-side references are never stored (M5).
- Excerpt (`excerpt.rs`, R15 · §6-2 L1003) — pulldown-cmark event walk that skips the `MetadataBlock` subtree, code blocks, tables (`ENABLE_TABLES` on so they are recognised) and HTML; the first paragraph's text, whitespace collapsed, capped (B16).

### 3-6. `validate`

```rust
pub struct Validator { schemas: RwLock<SchemaSet> }        // swapped as a whole; generation in SchemaSet
pub struct SchemaSet { pub generation: u64, per_backend: HashMap<String /* backend word */, HeldSchema> }
pub struct HeldSchema { pub source: SchemaOrigin /* Shipped | Cached */, pub cli_version: Option<String>, compiled: Arc<jsonschema::Validator> }

pub fn check_settings(doc: &SettingsDoc, set: &SchemaSet, installed: &dyn Fn(&str) -> Option<String>) -> SettingsVerdict;   // every raw field whose name a backend knows
pub fn check_merged(backend: &dyn SettingsCheck, layers: &[(LibPath, &Value)], set: &SchemaSet) -> Vec<(usize /* layer */, Reason)>;
pub fn check_skill(backend: &dyn SettingsCheck, mode: AgentOutputFormat, file_name: &str, folder: &str, text: &str) -> SkillFormat;  // passes YamlFrontMatter
pub struct SettingsVerdict { pub problems: Vec<(String /* field */, Reason)>, pub notices: Vec<(String, Notice)> }

pub trait SchemaFetch: Send + Sync { fn fetch(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String>; }
pub enum Fetched { NotModified, Body { bytes: Vec<u8>, etag: Option<String> } }
pub trait CliVersionRead: Send + Sync { fn read(&self, probe: &VersionProbe) -> Option<String /* stdout */>; }
impl LibraryService { pub fn refresh_schemas(&self, fetch: &dyn SchemaFetch, read: &dyn CliVersionRead); }
```

- For each raw field whose name `settings_check_for` knows: `raw_to_json` (port) → schema `iter_errors` → type · range error = ② with the line of that field; unknown key per `unknown_keys()`: `Ignore` = silent at any depth (incl. `additionalProperties:false` violations) · `Warn` = the notice `settings-unknown-key` (U4 · B8); no schema held = notice `settings-schema-unavailable`; schema older than the installed CLI ⇒ unknown keys become the notice `settings-schema-outdated` instead (P10-ADD · U3).
- Each value also passes `command_line_args` (port — a backend whose settings reach the command line refuses unsafe lines ②; others answer `Ok(vec![])`).
- A merged value (`merge` → `Some`) is checked as a whole; `check_merged` uses `merge_origins` (§3-9) so an error lands on the later asset row that brought the value (M14). The merge is never re-derived here (step-5 hand-over).
- jsonschema options are explicit: draft auto-detected; format validation on for known formats, unknown formats ignored, stated in code (§1-12).
- `refresh_schemas`: for every slot of `settings_checks()`, read the CLI version through `CliVersionRead` (outside every lock), compare with the held copy per `schema_source(version).refetch`, fetch through `SchemaFetch` (outside every lock), write the cache file atomically, compile, then **plug the new set in before drawing a ticket** and re-check every settings item as a `Catalog` change (§6 · ADR-0332 「영향」). Failure keeps what is held.

### 3-7. `resolve`, `refs`, `builtin`

```rust
pub struct Target { pub backend: Option<String>, pub mode: Option<AgentOutputFormat>, pub default_preset: bool, pub presets: Vec<AssetRef>, pub root: RootState }
pub enum RootState { Unset, Missing, Ok }
pub trait Lookup { fn item(&self, r: &AssetRef) -> Looked<'_>; }      // Present(&ItemView) | Missing | Unreadable(Reason)
pub fn resolve(target: &Target, lookup: &dyn Lookup, checks: Option<&dyn SettingsCheck>) -> Resolution;
pub struct Resolution {
    pub instructions: Vec<(String /* source path */, PathBuf)>,
    pub skills: Vec<(String /* name */, PathBuf, Vec<String> /* via */)>,
    pub raw_settings: Vec<(String /* preset */, Value)>,
    pub folders: Vec<ResolvedFolder>,                    // normalized, deduped by path_key, with via
    pub include_priming: bool,
    pub problems: Vec<Problem>,                          // Missing · Broken · Conflict · LibraryUnavailable — with via chains
    pub notices: Vec<Notice>,
}
impl Resolution {
    pub fn folder_paths(&self) -> Vec<PathBuf>;                         // phase 1: what to stat
    pub fn attach_folder_status(&mut self, st: &HashMap<PathKey, FolderStatus>);   // phase 2: ① lines
    pub fn blocked(&self) -> Option<LaunchBlocked>;
}
```

- **Pure** — no disk. Lookup is `Catalog` + the overlay of bytes the launch check read (§6), or `MemStore` in tests. Folder status arrives through phase 2 (the injected seam — ADR-0341 · §1-3).
- Expansion order, dedup at first place, cycle, per-preset tag order (G51), conflicts on loaded skills only, notices ① ② ③, folder lines, block ④ (a library preset attached while `root` ≠ `Ok`). What is loaded and which skill names are errors come only from `checks` (`loads` · `accepts_skill_name`).
- **Single-preset mode** (no agent — `backend: None`): everything is shown; a same-name skill pair is a conflict only if some backend would load both (U5 (a)): common × common, common × tagged, same tag × same tag; two different tags never conflict. "Some backend" ranges over `settings_checks()`, so a new backend is covered without change.
- `refs.rs`: `closure(preset) -> Vec<AssetRef>` (excludes `engram:/` and folder entries — ADR-0341) · `referrers_of(item) -> (Vec<LibPath>, Vec<AgentId>)` (presets from `Catalog.used_by`, agents from `ProfileRegistry::attachments()` with `same_path`) · `reference_count`. One walk, reused by 9a · 9b · T-73.
- `builtin.rs`: `Builtins::new(dir: Option<PathBuf>)`; `engram:/default` = `<dir>/presets/default.yaml` (a preset in library shape) etc. — file names B14. Missing dir → the built-in preset expands to nothing, `include_priming = false`, a log warning (M7 · B15). A library preset that lists `engram:/…` in `presets` = broken line (Δ1).

### 3-8. `folder` and `launch`

```rust
pub struct BoundedIo { limit: Duration /* 10 s — U2 */, list_cap: usize /* 10,000 — U2 */, inflight: Mutex<HashMap<(Op, PathKey), Arc<Pending>>> }
pub enum FolderStatus { Ok, Missing, NotADirectory, Unreadable(String /* diagnostic: OS error or "timeout" */) }
/// One computation's IO budget — started once (`now + limit`) and passed to every batch of that computation.
#[derive(Clone, Copy)] pub struct IoBudget { deadline: Instant }
impl BoundedIo {
    pub fn budget(&self) -> IoBudget;                                                          // start the 10 s budget of one computation
    pub fn stat_all(&self, paths: &[PathBuf], b: IoBudget) -> HashMap<PathKey, FolderStatus>;  // concurrent across keys; waits until b's deadline at most
    pub fn list_all(&self, paths: &[PathBuf], b: IoBudget) -> HashMap<PathKey, Result<FolderListing, FolderStatus>>;
    pub fn canonical_all(&self, paths: &[PathBuf], b: IoBudget) -> HashMap<PathKey, Option<PathBuf>>;
}
pub struct FolderListing { pub entries: Vec<agent::backend::settings_check::FolderEntry>, pub truncated: bool }
```

- **One limit, 10 s, for every bounded folder operation** — stat, listing, canonicalize (U2). No separate values.
- **Concurrent per computation** (main 2026-11-11): a call takes every distinct key it needs at once, starts one worker per key that has no call in flight (`std::thread::Builder` — a spawn failure is the value `Unreadable`), and waits on all of them until the **budget's deadline**. N unreachable folders therefore cost about 10 s in total, not N × 10 s. A key whose worker has not answered by the deadline is `Unreadable("timeout")`; the worker is abandoned, not cancelled; a later caller for the same key joins the pending result instead of starting another (§1-3 「Stat mechanics」 (ii)). Abandoned workers are bounded by the distinct unreachable keys.
- **One budget per computation** (main 2026-11-11): a computation that needs more than one IO batch — `check` (root + folder stats, then listings + canonicalizations that depend on them), a `resolve_*`, `prepare` (the folder instruction files) — calls `budget()` once at its start and passes the same `IoBudget` to every batch. A later batch gets only the time left; a batch that starts after the deadline answers every key it has not already got as `Unreadable("timeout")` without waiting. So the user-visible worst case is ≈ 10 s per launch check, however many folders and batches (what the user was told — U2). A key answered in an earlier batch, or still in flight from one, is reused, not asked again.
- **Over the limit = unreadable = block ①** for the whole launch (ADR-0341 decision 6, user re-confirmed 2026-11-11 「그냥 하나 막혀도 전체 막는게 좋겠지? 구지 무리하게 실행하는것보다는」).
- **Listing cap 10,000 entries**: past it the listing stops and is marked `truncated`; the preview says it is partial, notices are computed on what was listed, nothing blocks (U2). Read-only, links not entered, at least six levels under skill roots.

```rust
impl LaunchKitSource for LibraryService {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked>;
    fn prepare(&self, plan: LaunchPlan, id: AgentId, epoch: u32) -> Result<LaunchKit, PrepareError>;
    fn release(&self, id: AgentId, epoch: u32);
}
pub fn sweep_launch_dir(launch_root: &Path) -> SweepReport;   // boot — step 8 calls it before plugging the port
```

**`check`** (one read-lock take — §5-2 4 (d)):
1. `word = backend_word(&profile.command)`; `checks = settings_check_for(&word)`; `None` (a backend with no settings check, e.g. Shell) ⇒ the Noop plan, `real: false` (B13). `mode = output_format`; `loads = checks.loads(mode)`; `codes = checks.notice_codes(mode)`.
2. Note the three numbers (change generation · root generation · swap-in count) from the gate.
3. **Outside every lock**: start the computation's `IoBudget`; stat the root (bounded, on that budget); read the attached preset files from disk and, as they parse, the files they reach (presets · settings · `SKILL.md` · instruction existence) — the fresh-bytes overlay; read the built-in priming text.
4. **Read lock once**: `resolve` over `Catalog` + overlay with `checks`; settings verdicts for the attached settings; release.
5. **Outside every lock, all folder IO for this check under one `IoBudget`** started at step 3 (the root stat is its first use — §3-8 「One budget per computation」 · §6): phase-2 folder stats; listings of each folder entry, fed to `folder_load_set(…, YamlFrontMatter, mode)` — its `loads` give the instruction file and extra roots, its `omitted` gives a `codes.folder_omitted` notice; when `native_skill_roots(places, mode)` is non-empty, the same-name comparison (library skill names + folder `skill_names` + `root_skills` over each existing native root, every `SKILL.md` canonicalized and kept once per canonical path — step-6 hand-over (a)) gives one `codes.same_name` notice per name; `command_line_args` + `command_line_fits` with `cwd` = the folder canonicalized as the manager does and `instructions` = the text this mode loads; `working_folder_blocks(canonical cwd, mode)` (hand-over (d)); `folder_entry_blocks` per folder.
6. **Short write lock**: the conditional late merge of the overlay (§6).
7. Problems ⇒ `Err(LaunchBlocked)` in the §1-3 8 order; else `Ok(LaunchPlan { real: true, … })`. When `loads.instructions == BuiltinPrimingOnly`, the plan carries no instruction sources and the omitted ones go into a `codes.omitted` notice (hand-over (b)). Notices carry identity-only `args` (premise 12). The plan's `stamp` records the content tags `check` saw (B7).

**`prepare`**: create `<launch_root>\<id>-<epoch>\`; under the read lock, re-read each source and compare with the plan's `stamp` — gone or changed ⇒ `Stale`; write the merged instructions file (priming first when `include_priming`, then the plan's sources in order — a folder's instruction file read outside the lock, gone ⇒ `Stale` — §1-5 L318); `instructions_hash` = B9; `last_applied_instructions_hash = None` (hand-over (e)); `include_priming = true` ⇒ always an `instructions_file` (hand-over (e)). Other failures ⇒ `Failed`.

**`release`** / **`sweep_launch_dir`**: walk the volatile folder depth-first without following links; every entry `StdLaunchFs.is_link` reports is detached with platform `unlink_dir` (any depth — hand-over (f)); then remove files and folders; errors are logged and left (best effort, bounded, no retry, no lock, no panic — §5-2 4 (c2) ⑤). Idempotent.

### 3-9. Agent additions (additive)

```rust
// backend/settings_check.rs — SettingsCheck
/// For each merged leaf (JSON pointer) the index of the layer whose value is in the merged value; list items carry
/// the layer that contributed them. `None` = no merged value (same as `merge`).
fn merge_origins(&self, layers: &[&Value]) -> Option<Vec<(String /* pointer */, usize)>>;

/// Whether the agent's working folder (canonicalized as the manager does) blocks this backend in `mode` (problem ②).
fn working_folder_blocks(&self, cwd: &Path, mode: AgentOutputFormat) -> Option<Reason>;

/// The notice codes this backend's launch check emits in `mode`; `None` = that notice does not apply.
fn notice_codes(&self, mode: AgentOutputFormat) -> NoticeCodes;
pub struct NoticeCodes { pub omitted: Option<&'static str>, pub folder_omitted: Option<&'static str>, pub same_name: Option<&'static str> }
```

- claude `merge_origins` = a walk of the same `merge_two` rule (`backend/claude/settings.rs:439`) that records origins — a test runs both on the same tables. codex = `None`.
- codex `working_folder_blocks` = `Terminal` → `cwd_reaches_codex_unchanged` moved into one `pub(super)` helper that `materialize` also calls (ADR-0353 — one rule), code `codex-working-folder-unsafe` owned by the codex backend · `StreamJson` → `None`; claude → `None`.
- codex `notice_codes` = `Terminal` → `{omitted: CODEX_TERMINAL_OMITTED, folder_omitted: CODEX_TERMINAL_FOLDER_OMITTED, same_name: None}` · `StreamJson` → `{same_name: CODEX_SAME_NAME_SKILLS, ..None}`; claude → all `None`. A new backend answers its own (B20).
- `LaunchPlan` gains `stamp: Option<String>` (opaque — `check` writes, `prepare` reads; Noop `None`) — B7; 9 literals (4 `manager/prelaunch.rs`, 3 `launch.rs`, 1 `backend/codex/transport.rs`, 1 `tests/codex_launch_live.rs`).
- `launch.rs`: the generic notice codes of U3 · U4 (`settings-backend-field-absent` · `item-other-backend` · `settings-schema-unavailable` · `settings-schema-outdated` · `settings-unknown-key`) get their own sentence in `own_wording`; they take the backend word from `args`, so no backend is named in the code.

### 3-10. base and platform additions

- **base** `file::is_temp_name(file_name: &str) -> bool` — true for `<anything>.tmp<pid>.<n>` with both numbers exact decimals as `temp_path` writes them; `<x>.corrupt.tmp<pid>.<n>` is true by the same shape. Table test per §4 L785. `temp_owner` is rewritten on top of it so the two cannot drift.
- **platform** `fs::normalize_folder(path: &Path) -> Result<PathBuf, NotAbsolute>` beside `path_key` (§1-2): strip `\\?\` / `\\?\UNC\` → absolute check (relative refused) → `\` separators → lexical `.`/`..` collapse → no trailing separator except a drive or UNC root. No disk access. Non-Windows: `/` separators, same lexical steps. Its doc cross-links `path_key`.

### 3-11. The service

```rust
pub struct LibraryValues {
    pub registry: Arc<ProfileRegistry>,
    pub root: Option<PathBuf>,                  // 9a replaces with LibraryConfig
    pub builtin_dir: Option<PathBuf>,
    pub launch_root: PathBuf,                   // daemon\run\launch
    pub schema_cache_dir: PathBuf,              // daemon\cache\schemas
    pub shipped_schema_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,                  // NativePlaces.home for native_skill_roots
    pub env: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,   // daemon env (profile env applied on top per agent)
    pub io_limit: Duration,                     // 10 s (U2) — a value so tests can shorten it
}
impl LibraryService {
    pub fn new(values: LibraryValues, store: Arc<dyn LibraryStore>) -> Self;
    pub fn list(&self, folder: Option<&LibPath>) -> ListAnswer;               // first-scan join; read lock; safety net for one unknown path
    pub fn refresh(&self, folder: Option<&LibPath>) -> RefreshReport;          // ticketed scan (or join per ⑤); changed count + problems of re-read files + skipped links
    pub fn resolve_preset(&self, preset: &LibPath) -> Resolution;              // first-scan join; folder stats outside the lock
    pub fn resolve_agent(&self, id: AgentId) -> Option<Resolution>;
}
```

## 4. Data shapes on disk

Unchanged from the parent §1-2 except the writer (ADR-0358).

- **Kinds** — §1-2 table. `.yaml`/`.yml` with top-level `kind: preset` / `kind: settings`; broken or unknown `kind` = 「오류」 row; no `kind` = not ours (not listed; `refresh` reports 「kind 없음」 for re-read ones); `SKILL.md` directly in a folder = skill (name per the OS rule — `same_path`); any other `.md` = instruction; names starting with `.`, base temp names and links inside the library (U1) are not scanned.
- **Preset** — `kind` · `version` · optional `description` · `presets` · `instructions` · `skills` · `settings` · `folders` · any other key (kept untouched by edits). **Two item shapes** (M20): a string (common) or `{path, backend}` — only in `instructions` and `skills`; `backend` = any word the backend registry knows; a tag in `presets` / `settings` = broken line. `folders` = plain absolute strings only.
- **Settings** — `kind` · `version` · optional `description` · raw fields named by backend word (case ignored; today `claude` = the settings object, `codex` = a list of `key=value` strings — the shape is the port's knowledge).
- **Folders read/write** — read into `FolderRef { raw, normalized }`; a read never rewrites the file; `resolve` keys by `path_key(normalized)`; writes (9a) store the normalized form.
- **Edits rewrite the whole file** (ADR-0358 · §3-4 write): serde-saphyr serializer, 2-space indent, YAML 1.1-safe quoting, the original line ending and BOM kept; comments, blank lines, quoting and flow style are not kept; unmodeled keys and their order are kept; a multi-document file is refused.
- **Volatile layout** (written by `prepare` only): `<launch_root>\<id>-<epoch>\` + the merged instructions file (name B14); backend `materialize` adds the rest inside it.

## 5. Hand-over items → where they land

| Item | Source | Lands in | Unit |
|---|---|---|---|
| `format.rs` reads and writes `folders` | ADR-0341 in 7 | §3-4 (`PresetDoc.folders`, `PresetLists.folders`). "Goldens incl. comment preservation" no longer applies (ADR-0358); the golden pins the values and the unknown-key survival instead | P3 |
| `resolve.rs` collects folder entries (order · dedup over normalized paths · stat through the injected seam) | ADR-0341 in 7 | §3-7 two-phase `Resolution` | P7 |
| `folder.rs` (stat with timeout · bounded read-only listing) | ADR-0341 in 7 | §3-8 `BoundedIo` (U2) | P8 |
| The path normalizer | ADR-0341 in 7 | §3-10 platform `fs::normalize_folder` | P1 |
| Folder lines in the problem list and block ① (every backend and mode) | ADR-0341 in 7 | `attach_folder_status` → `Missing { item: "folder:<path>", code: folder-not-found }` | P7 · P9 |
| Folder omission notices (codex terminal today) | ADR-0341 in 7 | `check` step 5 → `codes.folder_omitted` from `FolderLoadSet.omitted` | P9 |
| `refs.rs` · `relink.rs` leave folder entries out | ADR-0341 in 7 | `refs` never sees `FolderRef` (not an `AssetRef`); `relink.rs` is 9a (B2) and inherits it | P8 |
| Per-asset error attribution from a new additive port method | step 5 | §3-9 `merge_origins` · `validate::check_merged` | P2 · P5 |
| Real-parser golden rows: YAML 1.1 scalars (`name: yes` · `on` · `off`) and non-string keys | step 5 | §8 `format` goldens through `YamlFrontMatter` + the codex port | P3 |
| Three claude preview divergences — fix here or keep recorded | step 5 | Kept recorded (B11) | — |
| (a) Produce the notices with the step-6 codes; same-name comparison with canonicalized `SKILL.md` paths | step 6 | `check` step 5, codes from `notice_codes` (§3-9) | P2 · P9 |
| (b) codex terminal plan: no instruction sources; merged file = priming only | step 6 | `check` step 7 (`loads.instructions == BuiltinPrimingOnly`) · `prepare` | P9 |
| (c) Check-time `command_line_args` · `command_line_fits` · `skill_format` through the real parser with repair rows | step 6 | `check` step 5 · `validate::check_skill` · goldens | P3 · P5 · P9 |
| (d) Working-folder counterpart on the canonicalized folder | step 6 · ADR-0353 | `working_folder_blocks` §3-9 · `check` step 5 | P2 · P9 |
| (e) `prepare` leaves `last_applied_instructions_hash` `None`; `include_priming = true` ⇒ an `instructions_file` | step 6 | `prepare` | P9 |
| (f) `release` detaches every directory link at any depth | step 6 | `release` · `sweep_launch_dir` | P9 |
| Same-name notice policy (launch + notice) | ADR-0355 | `check` step 5 | P9 |
| §2-4 4 single-preset tag conflict | parent open | U5 (a) — §3-7 | P7 |
| §2-4 8 dim-line wording | parent open | U3 (a) — §3-9 · §10-2 | P2 · P7 |
| ADR-0358 unknown keys survive an edit | ADR-0358 「영향」 | §3-4 write step 3 · §8 test | P3 |

**Deferred explicitly**: `relink.rs` · `edit.rs` · `config.rs` → 9a; `LibraryCommandHost` impl → 10a; `.git/info/exclude` · bundle ids → 9b; real fetch · version runner · refresh thread → 19; daemon wiring · boot sweep call → 8; folder-line wording on screen and the per-backend rows of the edit area → 13.

## 6. Concurrency and IO

The rule set is ADR-0324 / parent §1-4 flow ① 「잠금」 ①–⑤, implemented once in `gate.rs` + `lib.rs`:

- **One `std::sync::RwLock<State>`** (`State` = `Catalog` · root state · schema generation seen). Reads take it once; a write never happens while holding the read lock (no upgrade).
- **Scan-generation `Mutex` + `Condvar`** (a leaf — nothing else is taken while holding it; the library lock is never held while waiting on it): ticket counter (FIFO), scan in progress, changes in progress, change generation, root generation, swap-in count, first-scan state ("raised" = one full scan swapped in), queued refresh generations with their range. `notify_all` on every release.
- **Scan**: walk + parse **outside** the library lock; swap-in under the write lock **only if** the change generation is unchanged since the scan started (else rescan); the swap-in raises the swap-in count before releasing; it changes items inside its range only; inside the range "later read wins" by `read_seq`.
- **First-scan join** at the entrance of every `Catalog`-reading method (`list` · `resolve_preset` · `resolve_agent` · later 9a edits) — before any ticket, count or lock. `check` is **not** an entrance.
- **Refresh merging** (⑤): a `refresh` joins a covering generation that is queued and not yet walking; a covering walk already in progress ⇒ its own ticket. Before the first scan is raised, a folder `refresh` goes through the first-scan entrance (and re-checks "raised" on waking).
- **Conditional late merge** (④) — writers: `check`'s fresh bytes and `list`'s one-path safety net. Both note the three numbers **before** the disk read; in the short write lock: change or root generation moved ⇒ discard all; only the swap-in count moved ⇒ discard inserts of paths not in `Catalog`, keep updates of existing items when `read_seq` is later.
- **Schema re-check** = a change: plug the new set in → draw a ticket → count as a change → re-check settings items → raise the change generation inside the write lock. It does not trigger a first scan.
- **Folder IO — bounded, concurrent, never under the library lock** (U2 · main 2026-11-11): every stat · listing · canonicalize on a folder entry, a native skill root or the library root goes through `BoundedIo` with **one 10 s limit**. One computation (a `check`, a `resolve_*`, `prepare`, a problem list, tree ⛔ over many agents) starts **one 10 s budget** (main 2026-11-11) and gathers the distinct paths of each phase and runs them **at once**; every phase waits only until that same deadline. `check` has two dependent phases (root + folder stats, then listings + canonicalizations); the second runs on the time the first left. So N unreachable folders and any number of phases cost ≈ 10 s per computation, not N × 10 s and not 10 s per phase. One in-flight call per key; each distinct key once per computation; anything unfinished at the deadline = unreadable = block ①.
- **Library files** inside the root are read with the store directly (local by assumption — a network root is R4).
- **Pre-write checks** — `format::write::rewrite_lists` re-parses its own output and compares it with the edited tree before returning (§3-4 step 5); `FsStore::write_atomic` stats the parent first: missing ⇒ `NotFound` (nothing created). The gap between that stat and base's folder creation remains (B4).
- **Lock order** with the manager (ADR-0324): asset-library lock → manager locks, one direction; `check` holds no manager lock and calls only registry reads (`launch.rs:549-551`). `release` takes no library lock.

## 7. Library survey per implementation part (parent §5-1 0a rule)

| Part | Choice | Version (B10) | Why (one line) |
|---|---|---|---|
| `LibPath` | relative-path | 2.0.x (2.0.3 current) | `/`-pinned relative paths with `strip_prefix`/`join`; our own validation on top (§1-12). |
| YAML read · front matter | serde-saphyr (`deserialize` · `strict_booleans`) | 2.0.0 chosen · 2.1.1 current | Line/column errors, duplicate keys refused, budget caps; `serde_yaml` archived (ADR-0336). |
| **Preset write** | **serde-saphyr `serialize`** (`to_string_with_options`) over our `Node` tree | same crate | ADR-0358: no new crate; `yaml-edit` dropped (license text, single maintainer, comments not needed). `serde_json::Value` cannot be the tree here (premise 38). |
| Document tree | std (`Node` + a serde visitor) | — | serde-saphyr has no value type; `indexmap` + serde_json `preserve_order` would change serde_json for the whole workspace graph and still drop non-string keys. |
| Front-matter split | std (hand port of codex `extract_frontmatter`) | — | codex's exact rule; `gray_matter` differs (ADR-0336). |
| Schema check | jsonschema (`resolve-file`, `idna`) | 0.60.2 chosen · 0.61.1 current | No HTTP/TLS with defaults off; draft auto-detect; `instance_path()` for attribution. |
| Scan | walkdir | 2.5.0 (lock) | `follow_root_links` / `follow_links` / `filter_entry` give the U1 policy directly; `jwalk`/`ignore` rejected (ADR-0336). |
| Excerpt | pulldown-cmark (default features off) | 0.13.6 | Event stream makes skipping code/tables/HTML/front matter simple. |
| Instructions hash | sha2 | 0.10 (lock 0.10.9) | Stable across restarts; 0 new packages. |
| Atomic write · copy | base `file` | — | ADR-0335. |
| Path key · identity · junction · normalizer | platform `fs` | — | ADR-0325; normalizer added here. |
| Locks · gate | std `RwLock` · `Mutex` · `Condvar` | — | ADR-0324 (parking_lot rejected). |
| Bounded concurrent IO | std threads + `mpsc` waiting until one per-computation deadline (`IoBudget`) | — | No crate cancels a blocking `metadata`; a worker per distinct key, every batch of a computation waiting on the same deadline, is the parent's mechanics plus main's concurrency and budget (B17). |
| Volatile cleanup | std + platform `unlink_dir` + agent `StdLaunchFs::is_link` | — | Detach-first is the stated contract (§1-5 6). |
| Schema fetch · version read | seams only | — | Real ones = step 19 (ureq 3.5.0 — ADR-0336). |

New packages vs draft 1: `yaml-edit` · `rowan` · `countme` · `text-size` are gone; `nohash-hasher` (serde-saphyr `serialize`) is new (premise 22).

## 8. Test plan

Tests first per unit (TDD). Map to the parent §4 rows L779–L799.

### 8-1. Per unit

| Unit | Seam / harness | What it pins |
|---|---|---|
| P1 base | table | `is_temp_name`: `<n>.tmp<pid>.<k>` · `<n>.corrupt.tmp<pid>.<k>` true; no number · leading zero · `+` · overflow false; `temp_owner` unchanged. |
| P1 platform | table, per-OS expected values through the helper | `normalize_folder`: `\\?\D:\x` = `D:\x` · `\\?\UNC\s\x` = `\\s\x` · `D:\x\..\Rules\` = `D:\Rules` · drive/UNC root keeps its separator · relative refused · `D:\Rules` and `d:/rules` key alike after normalize + `path_key`. |
| P1 path | pure | `LibPath` refusal table · prefix swap · `AssetRef::parse` (`engram:/…` vs library; unknown built-in). |
| P2 agent | pure (both slots) | `merge_origins` agrees with `merge` on every existing merge table; codex `None`; `working_folder_blocks` = the moved `cwd_reaches_codex_unchanged` table (step-6 `materialize` tests unchanged); `notice_codes` per backend × mode; `own_wording` rows for the five new codes (backend word from `args`). |
| P3 format read | pure · golden | Item shapes round trip; tag in `presets`/`settings` = line problem; unknown tag word = line problem, a word added to a fake registry is accepted; `folders` relative/`LibPath` = line problem; lenient `kind`; error line numbers; YAML 1.1 scalars under `strict_booleans`; multi-document · mixed 0/2 indentation · duplicate keys · `!!tag` (§6-2 L989). |
| P3 codex skill golden | real `YamlFrontMatter` + `CODEX_SETTINGS_CHECK` | Upstream `parser_tests.rs` L56–85 rows · the research's pinned rows · repair rows (colon+space, `[` `{` `@` `` ` `` starts, unknown fields, trailing `#`, already quoted, block scalars) · `name: yes`/`on`/`off` · non-string keys · `description: 123` · BOM · whitespace-only block (M63 · §6-2 L1017). |
| P3 format write | pure · golden | **An unknown top-level key (and a nested unknown key inside an item map) survives an edit with its value and order** (ADR-0358); `description` untouched; each of add · remove · reorder · tag on · tag off · empty list · first item into `[]` · absent list key on all five lists incl. `folders`; CRLF in ⇒ CRLF out · LF ⇒ LF · BOM kept · no BOM ⇒ none; strings `yes` · `on` · `null` · `1.0` · `012` · `~` · `-` · `:x` re-read as the same strings; a non-string key survives; an alias is written expanded with equal values; a multi-document file is refused and left untouched; a forced serializer mismatch (test hook) ⇒ `preset-write-mismatch`, nothing returned; comments are gone (pinned, so a future change is visible). |
| P4 store/catalog | `tempfile` (real) + `MemStore` | Real scan skips `.trash`, dot names, temp names; a leftover temp file is neither overwritten nor deleted; `write_atomic` with the parent gone = `NotFound`, folder not recreated; `trash` is a same-root rename; **U1**: a root that is a junction is followed, a junction / symlink folder and a file link inside are not entered or listed and appear in `skipped_links`; Unsupported identity ⇒ `None`; re-read only on tag change or racy mark; preset-side reverse index; excerpt drops front matter · code · tables · HTML. Scan cost on a generated 5,000-item tree (`#[ignore]` measure — §6-2 L1004). |
| P5 validate | fake port answers · fake fetch · fake version read · fixture schemas · a fake third backend slot | Syntax ⇒ ② with line; type/range ⇒ ②; `Ignore` silent at any depth; `Warn` ⇒ `settings-unknown-key`; older schema ⇒ `settings-schema-outdated`; no schema ⇒ `settings-schema-unavailable`; merged-value error on the later asset's row; real claude schema compiles and passes under 0.60.x with format validation stated; refresh keeps the held copy on failure; a raw field named by the fake third backend is checked with no asset-library change. |
| P6 service/gate | `MemStore` with a scan counter · a gate that holds a scan · failure injection | `list` never scans after the first scan; first-scan join from every entrance; refresh merging only when the range covers; refresh before the first scan goes through the first-scan entrance; FIFO both ways; a scan that saw a change generation move rescans; late merge discard rules; schema re-check as a change; `refresh` returns skipped links. |
| P7 resolve | `MemStore` lookup · fake ports · fake folder status | Parent §4 L779 rows (D1 order · dedup · cycle · conflicts on loaded skills only · G51 per-preset tag order · only the agent backend's raw field · notices ①②③ · same reasons whoever launches · block ④); **U5**: common×common, common×tag, same-tag×same-tag conflict, claude-tag×codex-tag does not, and a fake third backend's tag behaves the same; folder entries (spelling dedup · ① for missing/unreadable on every backend and mode · ② where `folder_entry_blocks` · no notice for a folder with nothing to load); built-in missing ⇒ no priming, no block. |
| P8 refs/folder | `MemStore` · in-memory registry · fake slow ops | Closure excludes `engram:/` and folders; agent referrers via `same_path`; `BoundedIo` (shortened limit, fake ops that sleep): **N slow keys finish in ≈ one limit, not N** (total elapsed < 1.5 × limit) · **two phases share one budget** — phase 1 (stats) answers at ≈ 0.9 × limit (≈ 9 s at the real 10 s), phase 2 (a listing that would take 0.5 × limit) is cut at the shared deadline ⇒ `Unreadable("timeout")`, total elapsed < 1.1 × limit · a phase started after the deadline returns at once with every unanswered key `Unreadable("timeout")` · a key already answered in phase 1 is not asked again in phase 2 · timeout ⇒ `Unreadable("timeout")` · a second caller joins the pending call · one op per key per batch · listing cap ⇒ `truncated` · links not entered. |
| P9 launch | `FsStore` + `tempfile` · real ports · fake `BoundedIo` timings | `check` blocks (each reason) and passes; a priming-only mode plan has no instructions; a priming that does not fit ⇒ `Broken`; working-folder rule on the canonicalized folder (a junction cwd); same-name notice with a canonical duplicate removed; notice codes taken from `notice_codes` (a fake backend with its own codes); one unreachable folder blocks the launch with ①; `prepare` `Stale` on a vanished or changed source and a vanished folder instruction file; `include_priming` ⇒ file; hash stable; `release` detaches junctions at depth 1 and 3 and the library skill folder survives; idempotent; `sweep_launch_dir` the same; one end-to-end real kit → `backend::materialize` for both registered backends on a temp folder. |

### 8-2. The four CI gates and the `/qa` binding (P1)

1. Test line — a `ci.yml` backend-job step `cargo test --locked -p engram-dashboard-asset-library` (no `--test-threads=4`: no test spawns a child process).
2. Dependency cap — a gates-job step on the `cli gate 1` template (`ci.yml:1207`): `cargo tree --locked -p engram-dashboard-asset-library --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | awk '{print $1}' | sort -u` equals exactly {agent · asset-library · base · command · platform}.
3. Messaging isolation — `ci.yml:561` alphabet gains `asset_library`.
4. `use tauri` 0 — a gates-job step on the agent template (`ci.yml:539`), with `test -d crates/engram-dashboard-asset-library/src` first.

`.claude/skill-bindings/qa.md` 「모듈별 명령」: add `asset-library` to the row `base · command · protocol · messaging` (`cargo test -p engram-dashboard-<이름>`, no thread flag). No new 「돌릴 것 표」 row, no lane. The license gate is run locally in P1 and P3 (premise 25).

### 8-3. Review check — backend extensibility (every unit's code review)

- `rg -n -i '\b(claude|codex)\b' crates/engram-dashboard-asset-library/src` — every hit must be inside a `#[cfg(test)]` module, a doc comment that describes today's port answers, or built-in content; **no hit in a branch, a match arm, a string the code emits, or a field name**.
- No `enum` over backends; every backend-dependent branch reads a port answer (§1-1 list).
- Each unit that branches on backend behaviour has a test with a **fake third backend** (a fake `SettingsCheck` with its own word and codes) proving the path works without change (P5 · P7 · P9 rows above). The fake is injected through the lookup used for `settings_check_for` (a test seam in `validate`/`launch` — the production path calls the agent registry).
- Main may later turn the `rg` line into a CI gate once tests live in separate files (B21).

## 9. Implementation units

Sequential unless marked. Each ends green: build, its tests, full regression, the four gates. Sizes are rough (prod / tests, lines). Draft 1's P3 (read) and P4 (edit) are merged: with ADR-0358 the writer is ~200 lines over the reader's own parse, and the two share `Node` and the parse options.

| Unit | Files | Size | Ends green because |
|---|---|---|---|
| **P1** skeleton + leaves + gates | base `file.rs` · platform `fs.rs` + `fs/normalize.rs` · new crate `Cargo.toml` · `lib.rs` (header) · `path.rs` · root `Cargo.toml` · `Cargo.lock` · `ci.yml` (4 gates) · `qa.md` | 450 / 450 | Nothing calls the crate; base/platform additions are new functions. |
| **P2** agent port additions | `backend/settings_check.rs` · `backend/claude/settings.rs` · `backend/codex/settings.rs` · `backend/codex/launch.rs` (rule → shared helper) · `launch.rs` (`stamp` field, `own_wording` rows) · the 9 `LaunchPlan` literals | 300 / 350 | Additive methods for both slots; `materialize` keeps the same rule; goldens unchanged. **Independent of P3–P4** — may run beside P3. |
| **P3** format: read · front matter · write · codex golden | `format/mod.rs` · `read.rs` · `frontmatter.rs` · `tree.rs` · `write.rs` | 900 / 1,350 | Pure functions + goldens. Needs P1. |
| **P4** store · scan · catalog · excerpt | `store.rs` · `catalog.rs` · `excerpt.rs` | 800 / 900 | Uses P3's parsers; no service yet. |
| **P5** validate | `validate.rs` · fixtures (claude schema; codex schema if premise 36 resolves) | 650 / 850 | Needs P2, P3, P4. |
| **P6** service + gate | `gate.rs` · `lib.rs` (`LibraryService`, `list`, `refresh`, schema re-check) | 900 / 1,100 | Concurrency tests with held scans; nothing outside calls it. |
| **P7** builtin + resolve | `builtin.rs` · `resolve.rs` · `lib.rs` (entrances) | 900 / 1,250 | Pure resolve over `Lookup`; entrances reuse P6's join. |
| **P8** refs + folder | `refs.rs` · `folder.rs` · `lib.rs` (re-exports) | 500 / 650 | Pure walk + bounded concurrent IO with fakes. |
| **P9** launch | `launch.rs` · `lib.rs` (`BoundedIo` wiring) | 900 / 1,150 | `impl LaunchKitSource` tested end-to-end on temp folders; no daemon caller. |

**Interfaces between units** (frozen when the earlier one lands): P1 → `LibPath`/`AssetRef`/`PresetItem`/`FolderRef`, `is_temp_name`, `normalize_folder`. P2 → `merge_origins`, `working_folder_blocks`, `notice_codes`/`NoticeCodes`, `LaunchPlan.stamp`, the five new codes' wording. P3 → `parse_preset`/`parse_settings`/`sniff_kind`/`YamlFrontMatter`/`library_skill_name`/`conflict_key`/`rewrite_lists` (the last used by 9a only). P4 → `LibraryStore`/`ScanListing`/`ContentTag`/`Catalog`/`Item`. P5 → `Validator`/`SettingsVerdict`/`check_merged`/`check_skill`/`SchemaFetch`/`CliVersionRead`. P6 → `LibraryService::new`, the gate API (`first_scan_join`, `with_read`, `late_merge(numbers, f)`), `list`/`refresh`. P7 → `Lookup`/`Target`/`Resolution` (two phases). P8 → `BoundedIo`, `refs::*`. P9 → `LaunchKitSource` impl, `sweep_launch_dir`.

Total ≈ 6,300 prod / 8,050 tests (draft 1: 6,500 / 8,500). One commit per unit (B18). After the units: `/engram-review code` per unit (P6 and P9 deep), `/engram-qa`, doc pass (§12). *Stop (after)* per the parent.

## 10. Decisions

### 10-1. Already decided (cite)

| What | Where |
|---|---|
| Separate crate, name, dependency set, no test feature, no network, four gates | ADR-0311 · parent §1-4 L163–L168 |
| Reading = serde-saphyr `strict_booleans` (`yes` is a string) | ADR-0336 · G49 |
| **Preset edits rewrite the whole file with serde-saphyr; comments, blank lines, quoting not kept; `yaml-edit` dropped; unmodeled keys must survive** | **ADR-0358** (user 2026-11-11 「그냥 주석 없어도 되니깐 표준으로 하자.」 — U6 (c)) · supersedes ADR-0333 |
| Scan = walkdir; skip dot names and base temp names; never delete temp files | ADR-0336 · ADR-0335 · G40 · M56 · M65 |
| Locking rules | ADR-0324 · parent §1-4 flow ① |
| Settings check in process; type/range blocks; claude unknown silent; codex unknown ⚠; shipped set + background refresh | ADR-0332 · P10 · P10-ADD |
| Per-preset tag order | G51 · parent §1-3 1 |
| Folder entries: absolute only, normalized, `path_key` dedup, block ① on every backend/mode, unreadable = ① | ADR-0341 · user 2026-11-04 |
| Same-name codex skills across roots ⇒ launch + notice; library-vs-library = conflict | ADR-0355 |
| Wording of the three step-6 codes; facts only | ADR-0357 |
| A codex terminal priming the line cannot carry blocks (real kits) | ADR-0353 · step 6 U1 (a) |
| Built-in missing ⇒ start without priming, no block | M7 · ADR-0327 |
| Kebab-case reason codes | codebase convention (premise 29) |
| **Backend extensibility** — tags, raw-field names and selection are strings matched through the registry; no backend literal in asset-library outside tests and built-in content | User 2026-11-11 (§1-1) |
| **U1 = (a)** — follow the root if it is a link; do not enter links inside the library; `library.refresh` names the skipped links | User 2026-11-11 「애초에 바깥폴더를 못넣게 했잖아. 근데 어찌어찌 들어가더라도 건너띄는게 좋지 않음?」. Convention (main 2026-11-11): generic walkers (walkdir default, ripgrep) do not follow links; skill loaders (codex `codex-rs/ext/skills/src/loader/discovery.rs`, the Claude Code CLI) do follow directory links — (a) differs from the loaders on purpose: G37 (outside content is copied in, not used in place) and 9b's move/delete would otherwise reach the far side of a link. Rejected: (b) follow every link · (c) list links as an error row. |
| **U2 = one 10 s limit; folder IO concurrent; over the limit = unreadable = block ① for the whole launch; listing cap 10,000 (truncate + mark partial, no block)** | User 2026-11-11 (changed from draft 1's 2 s / 5 s; 「너무 디테일하게 하지말고 심플하게 가는게 좋아보이는데?」; block rule re-confirmed 「그냥 하나 막혀도 전체 막는게 좋겠지? 구지 무리하게 실행하는것보다는」). Concurrency across paths = main 2026-11-11. Convention: cap — VS Code search `limitHit`, codex `MAX_SKILLS_ENTRIES_PER_ROOT` + `inventory_complete`; timeout — none found (Claude Code issue #48361 hangs on an offline drive). |
| **U5 = (a)** — in a single preset, a same-name pair conflicts only if some backend would load both; two different tags never conflict | User 2026-11-11 「A」. Closes parent §2-4 4. |
| **U6 = (c)** — drop `yaml-edit`, rewrite with serde-saphyr | ADR-0358 (options (a) list as synthesized and (b) hold the dependency rejected by the user) |

### 10-2. Main decided or can decide (recommendation + reason)

| # | Question | Decision / recommendation | Reason |
|---|---|---|---|
| **U3** | New notices and their carriage | **Decided by main 2026-11-11 (user: wording detail) = (a)**: the launch reply carries them (once per agent — R58) and the views show them, each with its own fact-only Korean sentence in `own_wording`. Codes: `settings-backend-field-absent` (「알림: 설정 <이름>에 <백엔드> 칸이 없어 실리지 않았습니다.」) · `item-other-backend` (「알림: <백엔드> 전용 항목이라 실리지 않았습니다: <이름>」) · `settings-schema-unavailable` (「알림: 설정 <이름>의 스키마를 확인하지 못했습니다(스키마 없음).」) · `settings-schema-outdated` (「알림: 설정 <이름>의 일부 키는 이 판 스키마로 확인하지 못했습니다.」). Rejected: (b) views only · (c) generic wording | The parent's design (§1-3 6 — ① ② ③ are one list carried by the launch); once-per-agent stops repetition; the generic 「의도된 동작입니다」 is wrong for a missing schema. `<백엔드>` comes from `args`, not code (§1-1). Closes the dim-line half of §2-4 8. |
| **U4** | codex-style unknown setting key | **Decided by main 2026-11-11 = (a)**: notice `settings-unknown-key` (「알림: 설정 <이름>에 스키마에 없는 키가 있습니다: <키>」), emitted whenever a backend's `unknown_keys()` is `Warn` — not keyed to codex. Rejected: (b) not shown · (c) a warning category (ADR-0331) | ADR-0332 keeps it ⚠, ADR-0331 has no warning category; a notice is the only non-blocking channel. Amends ADR-0331 「영향」 (its open mismatch). |
| B1 | File split and names | `format/` folder (read · frontmatter · tree · write); `gate.rs`; `excerpt.rs`; keep `command` in the manifest now | One reason per file; gate ② final from day one. |
| B2 | What of §1-4's file list is step 7 | `relink.rs` · `edit.rs` · `config.rs` · the profile-edit late cell → 9a; `ops.rs` → 9b | They write files or profiles; step 7 is read + launch + the writer primitive. |
| B3 | Storage seam completeness | Declare and implement all methods now (`rename` · `copy` · `trash` thin, tested) | Seam frozen once. |
| B4 | Missing-parent write | Stat the parent, `NotFound` if missing; record the narrow gap | M65 "implementation judgment". |
| B5 | Lenient `kind` and item shapes | Line scan for a top-level `kind:` before the typed parse; items via a value enum, not `untagged` | `Spanned` is lost inside `untagged`/`flatten` (§6-2 L1018). |
| B6 | Working-folder rule's port shape | `working_folder_blocks(cwd, mode)`; the codex rule in one helper for both callers | One rule (ADR-0353); mirrors `folder_entry_blocks`; also closes the duplication T-92 names for this guard. |
| B7 | How `prepare` detects "changed" | `LaunchPlan.stamp: Option<String>` (opaque content tags) — 9 literals | Stateless and explicit; the alternative (a stash keyed by agent id) hides state across calls. |
| B8 | Code names (asset-library's own) | Blocks: `not-found` · `folder-not-found` · `yaml-syntax` · `kind-unknown` · `wrong-kind` · `preset-cycle` · `builtin-in-library-preset` · `backend-tag-unknown` · `folder-path-relative` · `folder-path-cli-unsafe` · `settings-schema` · `skill-name-invalid` · `skill-name-conflict` · `library-unavailable`; writer `preset-write-mismatch` · `preset-multi-document`; notices = U3 · U4. Port codes pass through unchanged and are owned by the backend (e.g. `codex-working-folder-unsafe`, `codex-skill-*`, `command-line-too-long`) | Kebab-case; one name per cause; no backend name in asset-library's own codes (§1-1). |
| B9 | Instructions hash (T6) | SHA-256 hex of the merged file bytes (sha2 0.10) | Stable across restarts; in the lock. |
| B10 | Versions | `jsonschema 0.60.x` (ADR-0336); serde-saphyr: the current 2.x **only after** the P3 goldens (read and write) pass on it, else pin `=2.0.0`; `relative-path 2.0`; `sha2 0.10` | A reader/writer bump is cheap to verify with the goldens. Report the resolved versions (CLAUDE.md 「의존성」). |
| B11 | Step-5 claude preview divergences | Keep recorded (tracking) | Agent code, preview-only. |
| B12 | T-96 (no claude `--add-dir` count) | Not in step 7; stays tracked | Needs a measurement first. |
| B13 | A backend with no settings check (Shell) in `check` | Noop plan (`real: false`) | It takes no preset (§1-5); argv byte-identical. |
| B14 | `builtin\` names and merged file name | `presets/default.yaml` · `instructions/priming.md` · `settings/default.yaml`; merged = `instructions.md` | Library shape (G1); step 17 creates them. |
| B15 | Built-in missing | Log a warning; no notice | M7; the release zip always ships it. |
| B16 | Excerpt rule | First paragraph text, whitespace collapsed, 200 characters | The client tunes display length. |
| B17 | Bounded IO mechanism | Worker per distinct key, one in-flight per key, abandoned workers joined by later callers; **one 10 s budget per computation shared by all its batches** (main 2026-11-11) | The parent's mechanics + main's concurrency and budget (U2): the user was told "however many folders, at most about 10 s". |
| B18 | Commit granularity | One commit per unit, `S25: feat(asset-library): …` | Each unit reviewable and green alone. |
| B19 | Version runner and fetch | Seams only; real ones in 19 | ADR-0311 decision 4. |
| B20 | Notice codes without backend knowledge | `SettingsCheck::notice_codes(mode)` (§3-9); asset-library emits a code only when the port gives one | Draft 1 would have picked the three codex-named codes by knowing the backend (§1-1). |
| B21 | Make §8-3's `rg` a CI gate | Later, once tests can be excluded mechanically; review check now | Inline `#[cfg(test)]` modules make a file-level `rg` gate noisy. |

### 10-3. User decisions needed

None open. (History: draft 1 asked U1–U6; U1 · U2 · U5 · U6 were answered by the user on 2026-11-11, U3 · U4 were left to main — §10-1 · §10-2.)

### 10-4. Open items touching step 7

| Item | Must step 7 resolve it? |
|---|---|
| Parent §2-4 4 | Resolved — U5 (a). |
| Parent §2-4 8 (notice wording) | Dim lines and schema notices: resolved — U3. Folder-line wording: no (13). |
| ADR-0331 「영향」 codex unknown-key mismatch | Resolved — U4 (a); the ADR text is amended in the doc pass (§12). |
| Parent §2-4 1 · 2 · 6 · 7 | No — 17 · 8 · 11 · 19. |
| Parent §6-2 rows tagged "step 7" (L989 unrun YAML shapes — now for the reader and the round trip, not `yaml-edit` · L1003 excerpt · L1004 scan cost · L1009 `strict_booleans` · L1015 FAT identity · L1016 walkdir/junctions — U1 · L1017 codex check parity · L1018 schema/Spanned · L1019 stat timeout — U2) | Yes — measured by §8 tests; results go back to §6-2. |
| Parent §6-5 6 · 10 · 11 (comment gaps, gate refusal, flow-list comments) | Moot under ADR-0358 — strike in the doc pass. |
| Tracking T-96 · T-101 · T-92 · T-62 | T-96/T-101 no (B12); T-92 partly closed by B6; T-62: the new packages' notices regenerate (no synthesized entry expected — premise 25). |

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | The gate is the hardest code in S25 and its bugs are races | P6 alone, deep review, held-scan and injected-order tests (parent §4 L783); rules already written (ADR-0324). |
| R2 | serde-saphyr vs codex's serde_yaml 0.9.34 edge differences | The P3 codex golden; record divergences that cannot be closed (§6-2 L1017). |
| R3 | Per-file `file_identity` makes a large first scan slow; the bus deadline (30 s) may bite | P4 measures 5,000 items; the first scan does not block launch; report before 10a if slow. |
| R4 | A library root on a network share blocks catalog reads on IO | Out of support this edition (default root is local); `BoundedIo` covers the root stat and folder entries only. Note in parent §6-3. |
| R5 | A whole-file rewrite surprises a user who hand-edits presets (comments vanish, layout changes) | Decided (ADR-0358); `engram help library` and the priming should say an edit through Engram rewrites the file (10a / 17 text — §12). |
| R6 | The serializer writes a value that re-reads differently (data loss) | The round-trip check refuses the write (§3-4 step 5); the P3 golden covers ambiguous scalars, numbers, non-string keys, unknown keys. |
| R7 | Abandoned IO workers pile up on many distinct unreachable paths | Bounded by distinct keys; one in-flight per key; logged. |
| R8 | (Closed as stated in draft 2 — one budget per computation caps `check` at ≈ 10 s.) **Residual**: a folder whose stat answers late (e.g. a share that wakes after 9 s) leaves little of the budget for its listing; the listing is then cut at the deadline and that folder counts as unreadable ⇒ block ①, although it exists. The user sees 「폴더를 찾을 수 없음」 for a slow but reachable share and a retry usually passes (the share is awake by then). | Accepted as the cost of a fixed ≈ 10 s cap (U2 — block rather than run partly). The diagnostic in the log says "timeout" so the case is visible; P9 measures how often a real slow share hits it. Only folders whose listing the backend needs are affected (a folder entry with nothing listed after the stat — claude today — has no second phase). |
| R9 | The real claude schema does not compile or pass on jsonschema 0.60.x | P5 finds out first; fallback = syntax-only + `settings-schema-unavailable`; report. |
| R10 | A backend added later needs a port answer asset-library does not yet ask for | §1-1 lists every port answer used; §8-3 fake-third-backend tests show the gap early. |
| R11 | A user expects linked content to work (U1) | `refresh` names skipped links; decided by the user. |
| R12 | Step 7 is large (≈ 14k lines incl. tests) | Nine green units; P2 can run beside P3. |

## 12. Hand-over to the parent TRD / ADR changes

Main copies these after landing (the parent TRD lines that change because of ADR-0358 and the 2026-11-11 decisions are listed first):

- **§1-2 L81** (the preset-file heading: "Engram's rewriting = an editor that keeps hand-written comments · shape … `asset_library::format` · T2") — rewriting = a whole-file rewrite with serde-saphyr's serializer; comments, blank lines, quoting not kept; unmodeled keys kept (ADR-0358). **L84** (the example's first comment, 「손으로 단 주석은 Engram 이 고쳐도 대개 남는다 …」) — replace with a comment saying an Engram edit rewrites the file and drops comments.
- **§1-4 `format.rs` row (L192)** — replace the `yaml-edit =0.3.5` text, the three conditions, the reorder detour and the pre-write gate by: read = serde-saphyr; write = parse into the order-keeping `Node` tree, replace the five lists, serialize, re-parse round-trip check (`preset-write-mismatch`), keep line ending and BOM, refuse multi-document files; the codex front-matter parts unchanged.
- **§1-12** — the `yaml-edit` row → "Rejected (ADR-0358)"; the serde-saphyr row: features `["deserialize","serialize"]`, uses = reading **and** rewriting presets, new indirect package `nohash-hasher`; version notes (B10).
- **§4 L784** (`format` test row) — the comment-preservation goldens become the §8-1 P3 write rows (unknown-key survival, values, line ending, BOM, ambiguous scalars, multi-document refusal, round-trip refusal).
- **§5-1 0b (L828)** — append: result superseded 2026-11-11 by ADR-0358 (the spike stays as background).
- **§5-2 step 7 (L893)** — "preset edit writing = `yaml-edit =0.3.5` + three conditions (0b · M61 — including the pre-write re-read gate · reorder detour …)" → "preset writing = whole-file rewrite with serde-saphyr (ADR-0358 — unmodeled keys survive; round-trip check)"; add the backend-extensibility rule and the notice-codes port method; then a "Step 7 landed" block.
- **§6-2 L989 · §6-5 6 · 10 · 11 · §6-3 (the gate-blind shape rows)** — moot under ADR-0358; the unrun shapes are re-pointed at the reader and the round trip.
- **§1-3 「Folder entries in resolution」 · §1-4 `folder.rs` row** — one 10 s limit for stat · listing · canonicalize; IO concurrent across paths, and one 10 s budget per computation shared by all its batches — a later batch runs on the time left, anything unfinished at the deadline = unreadable = ① (main 2026-11-11; worst case ≈ 10 s per launch check); listing cap 10,000 with `truncated` (U2).
- **§1-2 kind table / §1-4 `store.rs` row** — links inside the library are not entered or listed; the root link is followed; `library.refresh` names the skipped links (U1).
- **§1-3 5 · §2-4 4** — closed by U5 (a). **§2-4 8** — the U3 codes and wording. **ADR-0331 「영향」** — the codex unknown-key mismatch closed by U4 (a).
- **§1-5 settings-check port** — three additive methods `merge_origins` · `working_folder_blocks` · `notice_codes` (B6 · B20); `LaunchPlan.stamp` (B7); the design rule of §1-1 (backend extensibility) as a paragraph under 「Backend settings-check port」.
- **Step 13 (client) — UI requirement from the user (2026-11-11)**: in the preset edit area, per-backend rows/fields (a backend tag on an item, a backend's raw field in a settings asset) appear only when the user adds them — not one column per backend always shown. User: 「ui 기준으로는 모든 칸에 다 나오면 보기 불편하니깐 추가해야지 나오는 형태? … 이건 ui 할때 자세히 해도 될듯」. Details are designed in 13; the server side already allows any registered backend word (§1-1), so the list of choices comes from the backend registry, not from the screen.
- **Steps 8 · 17 · 19 · 9a · 10a** — step 8 calls `sweep_launch_dir` before plugging the port and passes `LibraryValues` (incl. `io_limit` = 10 s); step 17 uses the B14 names; step 19 implements `SchemaFetch` · `CliVersionRead` and calls `refresh_schemas`; 9a builds `preset.*` on `rewrite_lists`; 10a/17 text — `engram help library` and the priming say an Engram edit rewrites the preset file (R5).
- **ADR candidates (`/engram-adr`)** — U1 (links inside the library are skipped); U2 (one 10 s limit, concurrent folder IO — amends ADR-0341's "value = implementation judgment"); U3 + U4 (new notice codes and their carriage — amends ADR-0331 「영향」, extends ADR-0357's facts-only rule); backend extensibility for asset-library (§1-1) with B20 as its port consequence (ADR-0334 impact note).
- **CLAUDE.md (step 18 input)** — 「백엔드 모듈 맵」: the `asset-library` entry (what it holds · its four dependencies · no Tauri/network/test feature · no backend literal); 「빌드·검증 명령」: `cargo test -p engram-dashboard-asset-library` and the gate names; 「의존성」: the new packages (serde-saphyr with `deserialize`+`serialize` · jsonschema · relative-path · pulldown-cmark · walkdir direct · sha2 direct; indirect `nohash-hasher` · `granit-parser` · `annotate-snippets` · `encoding_rs_io`) — report to the user.

## 13. Review record

(empty — draft 2)
