# TRD — the asset-library crate: path · format · storage seam · scan · settings check · resolution · reference walk · built-ins · launch materials + gates (S25 step 7)

| Item | Value |
|---|---|
| **Status** | **Draft 3 · 2026-11-11** — review round 1 (deep) FIX applied: the 11 must findings, the recommended ones (§13), and the two decisions taken after the review — **D1** (one 5 s folder-IO budget per launch, shared by every check and the prepare of that launch — §3-8 · §10-1) and **D2** (the notices `settings-backend-field-absent` and `item-other-backend` are removed — §10-1). Draft 2 = `72d40d59` (U1–U6 applied, not reviewed). Draft 1 = the 2026-11-10 file (not committed). |
| **Canonical ADRs** | ADR-0311 (the crate, its dependency set, its four gates) · ADR-0324 (locking: one `RwLock`, scan-generation ticket FIFO, conditional late merge) · ADR-0332 (settings check in process) · **ADR-0358** (preset edits rewrite the whole file with serde-saphyr — supersedes ADR-0333, amends ADR-0336 and ADR-0338) · ADR-0334 (launch materials through ports; backend knowledge in `backend`) · ADR-0336 (server component choices, as amended) · ADR-0341 (folder entries) · ADR-0353 · ADR-0355 · ADR-0357 (step-6 duties of `check`) · ADR-0315 (references) · ADR-0325 (path key · file identity) · ADR-0327 (priming source = `builtin\`) · ADR-0328 (one merged claude `--settings`) · ADR-0329 (backend tags on preset items) · ADR-0331 (no warning category) · ADR-0335 (atomic write · no auto-cleanup of temp files) · ADR-0337 (codex materials) · ADR-0338 (draft + save — decision 6 writer amended). |
| **Line pointers** | Code anchors re-read at `60002c49`; HEAD is now `72d40d59`, which changes only `docs/` since (`git diff --stat 60002c49 72d40d59`), so every code pointer still holds. The pointers added in draft 3 (bus deadline, prelaunch flow, port read closures, serde-saphyr number paths) were read at `72d40d59`. Agent paths are under `crates/engram-dashboard-agent/src/` unless stated otherwise. |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 7 (L893), "ADR-0341 in 7" (L895), "Handed over by step 5" (L897), "Handed over by step 6" (L899); §1-2 (L62–L139), §1-3 (L140–L159), §1-4 (L161–L235), §1-5 (L237–L331), §1-12 (L560–L595), §4 (L773–L804), §6-2 (L984–L1019). |

## 1. Scope and relation to the S25 TRD

**What this step builds.** A new workspace crate `crates/engram-dashboard-asset-library/` (Rust `engram_dashboard_asset_library`) with the parts the parent assigns to step 7, plus the small additions outside it that those parts need:

- **inside the new crate** — `backends` (the backend-registry seam `Backends` — §1-1) · `path` (`LibPath` · `AssetRef` · `BuiltinId` · preset items) · `format` (YAML reading with serde-saphyr; the whole-file preset writer on serde-saphyr's serializer over an order-keeping document tree — ADR-0358; the real `FrontMatterParser`) · `validate` (settings check over the backend settings-check port; the schema set; the fetch and CLI-version seams) · the storage seam `LibraryStore` (`FsStore` · `MemStore`) and the scan · `catalog` (item table, content tags, racy rule, preset-side reverse index) · `refs` (the reusable reference walk — G37) · `folder` (bounded, concurrent folder IO on a per-launch budget) · the service `LibraryService` with the ADR-0324 locking (read lock · scan-generation ticket FIFO · first-scan join · refresh merging · conditional late merge) and the read entrances `list` · `refresh` · `resolve` (pure — expansion, tags, dedup, cycle, conflicts, problem list, notices, folder entries through an injected stat seam) · `builtin` (`engram:/…`) · `launch` (`impl LaunchKitSource for LibraryService` — `check` · `prepare` · `release` and their launch-scoped forms — and the boot sweep of `launch\`);
- **outside it** — base `file::is_temp_name` (M65) · a folder-path normalizer in platform `fs` (§1-2) · four additive methods and one additive field on the agent settings-check port (the merge-origin map handed over by step 5; the working-folder rule handed over by step 6 (d); the backend's notice codes, needed for backend extensibility — §1-1; whether a launch lists a folder entry and what it carries from it — §3-9) · one additive `LaunchPlan` field (B7) · the launch scope on `LaunchKitSource` (three default methods — D1 · §3-9) · the notice wording for the new codes in `launch.rs` (U3 · U4 · B22) · the four CI gates and the `/qa` binding row (M17).

### 1-1. Design rule — backend extensibility (user 2026-11-11)

User: 「나중에 백엔드 계속 추가될텐데 최대한 공용화로 하고(예를들어 ENUM 추가하면 모두 자동으로 되게 한다던가.)」. **Adding a backend to agent (a new `AgentCommand` variant, its `backend_word`, its `SettingsCheck` in `settings_checks()`) must make tags, settings raw fields, the selection of what an agent loads, the schema check and the notices work with no asset-library change.** Concretely:

- A backend tag and a settings raw-field name are **strings**; asset-library accepts a word only if the backend registry knows it, and compares case-insensitively through it. No `enum Backend` in asset-library.
- **The registry is reached through one seam, `Backends`** (§3-3a): `for_word(word)` and `all()`. Production = `AgentBackends`, which calls agent `settings_check_for` / `settings_checks()` (`backend/mod.rs:770-781`); tests inject a registry with a fake third backend. `format`, `validate`, `resolve`, `refs` and the service take `&dyn Backends` (the service holds an `Arc<dyn Backends>` from `LibraryValues`); **no asset-library code calls `settings_check_for` / `settings_checks()` directly** except `AgentBackends` (reviewed in §8-3).
- Everything that differs per backend or mode comes from a port answer: `loads` · `accepts_skill_name` · `merge` / `merge_origins` · `unknown_keys` · `raw_to_json` · `command_line_args` · `command_line_fits` · `skill_format` · `folder_entry_blocks` · `working_folder_blocks` · `folder_listing_at_launch` · `folder_load_set` (incl. its `carried` rows) · `native_skill_roots` · `root_skills` · `schema_source` · `version_probe` · and the notice codes (`notice_codes`, §3-9). asset-library branches only on those answers, never on the backend word.
- **No `claude` / `codex` literal in asset-library outside tests and the built-in content files** (reviewed in §8-3).

Draft 1 broke this in two places (§10-2 B20): asset-library would have chosen the three codex-named notice codes (`CODEX_TERMINAL_OMITTED` · `CODEX_TERMINAL_FOLDER_OMITTED` · `CODEX_SAME_NAME_SKILLS`, `launch.rs:350-360`) by knowing that the backend is codex, and the draft named the working-folder refusal code as if asset-library owned it. Draft 2 broke it in two more (review round 1): the registry was called as a global, so the fake-third-backend tests could not be written (must 10 → `Backends`), and `check` could only tell claude's natively loaded folder files from codex's carried ones by the backend (must 9 → `folder_listing_at_launch` + `FolderLoadSet.carried`). Prose that says "codex terminal" / "claude merged" describes what today's port answers produce, not branches in the code.

**Nobody calls the crate yet.** No daemon edge, no command, no manager wiring. Every launch stays Noop until step 8. Build and the full regression stay green because nothing outside the crate's own tests reaches the new code, and the agent additions are additive (existing callers and goldens unchanged; the default `LaunchKitSource` methods delegate to today's `check` / `prepare`).

**What it leaves to later steps:**

| Step | What stays there (not built here) |
|---|---|
| 18 | CLAUDE.md 「백엔드 모듈 맵」 · 「빌드·검증 명령」 · 「의존성」 lines (after user confirmation — M1). This step only gathers the text (§12). |
| 17 | The in-repo `builtin\` sources (「기본」 preset · priming `.md` · settings `.yaml`) and one shipped schema set per backend in the release zip; the dev · test · `ENGRAM_DATA_DIR` lookup rule (§2-4 1). Here `builtin\` and the shipped schema folder are constructor values; tests use fixtures. |
| 8 | Daemon wiring: `LibraryService` → manager as `LaunchKitSource`, `DataLayout` values, `daemon_id`, the boot call of the launch sweep **before** the port is plugged (§5-2 4 (c2) ⑥), **one `LaunchScope` per launch threaded through `check_before_create` → `prelaunch` (every attempt) → `prepare` (D1 — §3-9 · §12)**, the first-turn hash recording (step-6 hand-over to 8 (a)), removal of the debug launch-block hook (step-20 hand-over), the real CLI lane. |
| 9a | `config.rs` (`library.json` · `setRoot` · first-start default root · moving root generation), `edit.rs` (`preset.add/remove/reorder/setBackend/save/check` · `rev` with sha2), `relink.rs`, the profile-edit port cell, `newFolder` · `newFile`, the four block reasons through edits. Step 7 provides the writer (`format::write`) and the walk (`refs`) they use. |
| 10a | `LibraryCommandHost` (agent) and `impl LibraryCommandHost for LibraryService`; command declarations; `engram help library`. Step 7's inherent methods (`list`, `refresh`, `resolve_preset`, …) are what those bodies call. |
| 9b · 10b | `ops.rs` (move · rename · copy · delete with reference fixing), the `.trash` bundle naming in use, `.git/info/exclude`, restore, emptying (platform OS trash). Step 7 defines and implements the storage-seam methods those use (`rename` · `copy` · `trash` — B3). |
| 19 | The real fetch port (ureq) and the real CLI-version runner; the start-of-daemon schema refresh thread; the async-import gate (§2-4 7). Step 7 defines both seams and the refresh method, tested with fakes. |
| 11 · 13 | Notices in the webview; the folder-line wording (§2-4 8); the dim display of rows not loaded for the agent's backend (a view matter — D2); the preset edit area's per-backend rows shown only when added (§12 — user 2026-11-11). |

## 2. Premises checked against code

Re-read at `60002c49` (still valid at `72d40d59` — docs-only since); rows 39–46 read at `72d40d59`. **Bold verdict = the premise does not hold as the parent, a hand-over or draft 2 states it.**

| # | Premise (source) | Verdict | Evidence |
|---|---|---|---|
| 1 | Agent defines `LaunchKitSource { check, prepare, release }`; `check(&AgentProfile)`; `PrepareError { Stale, Failed }` (§1-5 L267) | True | `launch.rs:553-590` (trait) · `:525-532` (`PrepareError`) · `NoopLaunchKits` `:591-627`. `check` is documented read-only, may read the registry and `workspace_lock`, and sees its own reservation as `Preparing` (`:556-567`). A profile not yet in the registry also arrives here (pre-insert check — `manager/prelaunch.rs:28-29`). |
| 2 | `LaunchPlan` carries the folder list and the extra skill roots (ADR-0341 in 16) | True | `launch.rs:154-180` (`folders` `:174` · `extra_skill_roots` `:179`). **No field carries what `check` read** — `prepare` can detect a vanished source but not a changed one without one (§5-2 4 (c) "vanished **or changed** ⇒ `Stale`"; `launch.rs:568-569`). B7 (draft 3: a content digest — must 7). |
| 3 | The settings-check port has a merge but no per-asset attribution (step-5 hand-over) | True | `backend/settings_check.rs:49` `fn merge(&self, layers) -> Option<Value>` — no origin. claude `Some` (`backend/claude/settings.rs:76-78`), codex `None` (`backend/codex/settings.rs:133-135`). An additive method is needed (§3-9). |
| 4 | `FrontMatterParser` has `paths` · `subagent` · `skill` · `is_yaml_value`; only fakes implement it (step-6 hand-over (c)) | True | `settings_check.rs:229-238`. `grep "impl FrontMatterParser"` = test fakes only. The real one is step 7's. |
| 5 | The codex front-matter serde type lives in agent (M72) | True | `settings_check.rs:243-259` `SkillFrontMatter` / `SkillMetadata` (`Deserialize`, unknown keys ignored). |
| 6 | One registry of backend checks | True | `backend/mod.rs:770-781` `settings_checks()` (two slots today) · `settings_check_for(word)` (ASCII case-insensitive). asset-library reaches it only through `Backends` (§1-1). |
| 7 | `check` can judge the codex working-folder rule on the canonicalized folder (step-6 hand-over (d)) | **False today** | The rule is private: `backend/codex/launch.rs:162` `fn cwd_reaches_codex_unchanged` (called by `materialize` `:121`). `command_line_fits` does not apply it — `backend/codex/cmdline.rs:129-140` counts `--cd <cwd>` length only. An additive port method is needed (§3-9 · B6). |
| 8 | The skill-link layout is not visible outside agent (step-6 hand-over (f)) | True | `launch.rs:911-913` `SKILL_LINKS_PARENT` / `SKILL_LINKS_DIR` are `pub(crate)`; the doc (`:905-910`) says a release outside the crate must detach every directory link at any depth. |
| 9 | A public link test exists to reuse | True | `StdLaunchFs::is_link` (`launch.rs:674-700`, `symlink_metadata().file_type().is_symlink()` — name-surrogate reparse points incl. junctions); platform `fs::unlink_dir` (`platform/src/fs.rs:19`). |
| 10 | The manager canonicalizes `cwd` with `dunce` before `materialize` | True | `manager.rs:1566` `dunce::canonicalize(&profile.cwd).unwrap_or_else(|_| profile.cwd.clone())` (agent `Cargo.toml` `dunce = "1.0"`; lock `dunce 1.0.5`). `check` judges the same value (hand-over (d)) with the same canonicalizer and the same fallback (§3-8 step 5). |
| 11 | Notice codes and wording | True — **but the codes are backend-named constants in agent** | `launch.rs:350` `CODEX_TERMINAL_OMITTED` · `:354` `CODEX_TERMINAL_FOLDER_OMITTED` · `:360` `CODEX_SAME_NAME_SKILLS`; `notice_text` `:373` with `own_wording` `:394`; any other code gets the generic 「알림(<code>): 의도된 동작입니다.」 (ADR-0357 decision 3). The port hands the codes out (§3-9 `notice_codes` · B20). |
| 12 | Notice `args` carry identity only | True | `notice_fingerprint` `launch.rs:485` keys on `code` + canonical `args` (no `line`). A diagnostic in `args` would re-show a notice. |
| 13 | Profile-edit port shape | True | `profile_edit.rs:52-88`. Step 7 calls none of them (`check` needs none; edits are 9a). |
| 14 | Profile fields | True | `profile.rs:126-128` `AttachedPreset { path: String }` · `:167` `default_preset` · `:171` `presets` · `:181` `last_applied_instructions_hash` · `:193` `notice_fingerprint`; `command` (`:155`) carries the variant and `output_format` (`AgentOutputFormat` — `Terminal` · `StreamJson`). |
| 15 | Agent-side references are read straight from the registry (M5) | True | `profile.rs:1075` `ProfileRegistry::attachments_of(id)` · `:1080` `attachments()` (pub). Writes are `pub(crate)`. |
| 16 | `base::file::is_temp_name` exists (M65 · brief) | **False** | Not in `base/src/file.rs`. Only private `temp_owner(file_name, target)` (`:515`) and `TEMP_MARK = ".tmp"` (`:498`); `temp_path` writes `<name>.tmp<pid>.<n>` (`:500-507`). Step 7 adds the public name-only test (§3-10). |
| 17 | base `write_atomic` creates a missing parent | True | `base/src/file.rs:338-348`. So `FsStore::write_atomic` checks the parent first (M65). |
| 18 | platform has `link_dir` · `unlink_dir` · `path_key` · `same_path` · `file_identity`; a folder-path normalizer exists | Partly — **no normalizer** | `platform/src/fs.rs:18-20` re-exports the five. `fs/path_key.rs:13-20` states the caller must pick the spelling. No normalizer anywhere (`base::path::normalize_spelling` `base/src/path.rs:14` only strips quotes and turns `\` into `/`). Step 7 adds it (§3-10). |
| 19 | The cmd guards | True | `platform/src/shell.rs:61` `path_is_cli_safe` · `:75` `text_is_cli_safe`. claude's `folder_entry_blocks` calls the first (`backend/claude/settings.rs:100-102`). |
| 20 | `file_identity` answers `Unsupported` for a FAT root | True (doc) | `platform/src/fs/identity.rs:20-27`. The scan takes it as "no identity", not a failure (§6-2 L1015). |
| 21 | Workspace dependency style | True | Root `Cargo.toml` has no `[workspace.dependencies]`; each crate states its versions. `members` = 10 entries (protocol · base · platform · agent · command · messaging · transport · daemon · cli · src-tauri), and the header comment says "10 members" — this step adds the 11th and updates that comment. |
| 22 | Which packages are new | True | `Cargo.lock` already has `walkdir 2.5.0` · `sha2 0.10.9` · `junction 2.1.0` · `tempfile 3.27.0` · `dunce 1.0.5`, and of serde-saphyr's `serialize` feature (`base64` · `num-traits` · `zmij` · `nohash-hasher`) `base64 0.22.1` · `num-traits 0.2.19` · `zmij 1.0.21`. **New**: `serde-saphyr` (+ its `deserialize` deps `granit-parser` · `annotate-snippets` · `encoding_rs_io`) · `nohash-hasher` · `jsonschema` · `relative-path` · `pulldown-cmark`. `dunce` becomes a direct dependency of the new crate (already in the lock). |
| 23 | The chosen versions are current (§1-12 · ADR-0336) | **Newer releases exist** | `cargo search` 2026-11-10: `serde-saphyr 2.1.1` (chosen 2.0.0) · `jsonschema 0.61.1` (chosen 0.60.2) · `relative-path 2.0.3` · `sha2 0.11.1` (lock 0.10.9) · `pulldown-cmark 0.13.6` · `walkdir 2.5.0` — unchanged. B10. |
| 24 | MSRV fits the toolchain | True for the measured versions | `rust-toolchain.toml` = 1.95.0. Registry copy `serde-saphyr 2.0.0` `rust-version = "1.89"`; `walkdir 2.5.0` none. 2.1.1 / 0.61.1: not determined. |
| 25 | New packages pass the license gate (`ci.yml:1628-1630` → `scripts/gen-third-party-licenses.mjs`) | True as far as checked | No new synthesized entry expected (ADR-0358). `serde-saphyr 2.0.0` ships `LICENSE-APACHE` · `LICENSE-MIT`; `nohash-hasher 0.2.0` ships both; `zmij` already in the lock. `granit-parser` · `annotate-snippets` · `encoding_rs_io` · `jsonschema` · `relative-path` · `pulldown-cmark`: not determined — the gate is run locally in P1/P3. |
| 26 | Gate templates | True | `ci.yml:557-567` messaging regex (no `asset_library`) · `ci.yml:1207-1235` the cli name-set gate (template for gate ②) · `ci.yml:539-555` the agent `use tauri` gate (template for ④) · per-crate test steps `ci.yml:286-301` (template for ①). `ci.yml:176-185` `cargo test --workspace` already runs the new crate. |
| 27 | `LibraryCommandHost` exists | False (expected) | `grep LibraryCommandHost` = 0 hits. It is 10a's. |
| 28 | `SpawnByCwd` runs the spawn on a blocking thread | True (pointer moved) | `daemon/src/connection_core.rs:1744` `tokio::task::spawn_blocking` around the create body. Step 8 re-checks when it plugs the service. |
| 29 | Reason codes are kebab-case | True | `backend/codex/cmdline.rs` `command-line-too-long` · `codex-priming-unsafe-char`; `backend/codex/settings.rs` `codex-config-line-invalid` … → B8 uses `folder-not-found`. |
| 30 | codex unknown keys are ⚠ but `Resolution` has no warning field | Resolved by U4 | `backend/codex/settings.rs:138-140` `UnknownKeys::Warn`. U4 (a) — a notice. |
| 31 | The codex terminal translation refuses a real plan with instruction sources (step-6 hand-over (b)) | True | `backend/codex/launch.rs:115-120`. Generic rule in `check`: when `loads(mode).instructions == BuiltinPrimingOnly`, the plan carries no instruction sources. |
| 32 | `LaunchArtifacts` has no notice channel | True | `launch.rs:917-955`. All notices come from `check`. |
| 33 | The claude preview divergences (step-5 hand-over) are asset-library's | **False** | They are in agent `backend/claude/settings.rs` `folder_load_set` (doc `:104-110`). B11. |
| 34 | `DataLayout` has the folders the service needs | Not checked | `daemon/src/data_dir.rs:207`. Step 8 / 9a. Step 7 takes plain paths. |
| 35 | jsonschema 0.60.2 compiles and passes the SchemaStore claude schema | Not determined | Only `jsonschema-0.59.0` is in the local registry. Pinned by a P4 test with a fixture fetched at implementation time. |
| 36 | The codex release-asset schema path `rust-v<ver>/config-schema.json` | Not determined | §6-2 L1018 "unconfirmed". P4 confirms before writing the fixture; until then those settings get syntax + the port's `-c` rules and the "schema not checked" notice. |
| 37 | serde-saphyr can write a value tree back to YAML (ADR-0358) | True | `serde-saphyr-2.0.0/src/ser/api.rs` `to_string` · `to_string_with_options`; `SerializerOptions` (`src/ser/options.rs:70-209`): `indent_step` · `compact_list_indent` · `empty_as_braces` · `yaml_12` (default `false` = YAML 1.1-safe quoting). **No line-ending or BOM option**, and **no value/document type of its own**. |
| 38 | `serde_json::Value` can be the document tree | **False** | `serde_json 1.0.150` without `preserve_order` → `Map` = `BTreeMap`: a rewrite would sort keys, and a non-string key cannot be held. The writer uses its own tree (§3-4). |
| 39 | Integers read back exactly | **False beyond 64 bits** | `serde-saphyr-2.0.0/src/de/deserializer.rs:893-909` tries `i64` (leading `-`) or `u64` only; an integer literal outside those ranges falls to `:910-918` `try_parse_float_incl_overflow` (`src/parse_scalars.rs:360-380` → `parse_yaml12_finite_float_fallback` `:280-298` = `str::parse::<f64>`), so `99999999999999999999` arrives as `visit_f64(1e20)` — the digits are lost **before** the tree exists, and a round-trip compare of the tree cannot see it. Must 4 (c) → the writer refuses (§3-4). |
| 40 | `.nan` / `.inf` read in a typeless parse | **Refused by default** | `src/de/options.rs:42-67` `NonFiniteFloatPolicy` (default sentinel) + `:628` `reject_non_finite_typeless_float: true` → `src/de/cfg.rs:45` = `Reject`; `deserializer.rs:559-590` `visit_typeless_float` errors. Every parse in this crate sets `PassThrough` (§3-4), so a `.nan` in an unknown key neither fails the read nor the edit; the tree compares floats by bits. |
| 41 | The bus `agent.spawn` reply deadline | 7 s | `daemon/src/command_delivery.rs:464` `DEFAULT_DEADLINE = 7 s` (10 − sweep 1 − margin 2; the socket entrance uses the same value). D1 sizes the launch IO budget to fit it. Draft 2's R3 said 30 s — wrong. |
| 42 | One launch calls `check` several times | True | Create path: `manager.rs:1345` `check_before_create` (`manager/prelaunch.rs:28-33`) → `prelaunch` up to `PRELAUNCH_ATTEMPTS = 2` (`prelaunch.rs:22`, `:50-77`) → `prepare` (`manager.rs:1549`). Up to three checks and one prepare per launch, all inside one bus reply. D1 → one `LaunchScope` (§3-9). The manager has source-text guard tests on these calls (`prelaunch.rs:1845-1938` count `.kits.check(` / `.kits.prepare(`) — step 8 updates them. |
| 43 | The port's `read` closures are bounded | **False** | `folder_load_set` / `root_skills` take `read: &dyn Fn(&Path) -> io::Result<String>` (`settings_check.rs:72-78`, `:116-121`) and call it synchronously per file: codex `backend/codex/settings.rs:776` (folder instruction file) · `:830` (each `SKILL.md`); claude reads every listed file but commands (test `every_file_but_commands_is_read`, `backend/claude/settings.rs:1367`). A read error becomes `unsure` (`UnsureWhy::Unreadable`, `settings_check.rs:366-370`), not a block. Must 1 → a bounded read (§3-8). |
| 44 | claude's folder load set is preview-only | True — **but nothing in the port says so** | `backend/claude/settings.rs:494-501` `native_kind` returns `CLAUDE.md` · `.claude/CLAUDE.md` · `CLAUDE.local.md` as `FolderLoadKind::Instructions`; parent `trd.md` L314 (folder-entries row) · L319 ("for claude only the preview uses it — claude reads the folder itself"). Draft 2 took "the instruction file" from `loads`, which for claude would double-load `CLAUDE.md` (once by `--add-dir`, once merged). Must 9 → `folder_listing_at_launch` + `carried` (§3-9). |
| 45 | `LaunchPlan` struct literals | **6, not 9** | `grep "LaunchPlan *{"`: `launch.rs:595` · `:1240` · `manager/prelaunch.rs:1310` · `:1436` · `backend/codex/transport.rs:12585` · `tests/codex_launch_live.rs:119`. `FolderLoadSet` literals = 9 (agent `backend/claude/settings.rs` · `backend/codex/settings.rs`) — P2 updates them for `carried`. |
| 46 | `library.list` rows carry a reference count | True | Parent `trd.md` L368 (row = path · kind · name · state · **reference count** · description). So `list` needs `refs` — unit order changes (§9 · must 8). |

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
dunce = "1.0"                                                            # the manager's canonicalizer (manager.rs:1566) — 0 new packages
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
- **Backend extensibility** (§1-1) — no backend word, no backend enum; every per-backend answer comes from `Backends` and the port.
- **No OS `cfg`** in the crate — OS behaviour comes from platform and `dunce` (platform gate 4 list unchanged).

### 3-2. Module map

| File | Holds | Unit |
|---|---|---|
| `lib.rs` | Crate header (invariants, gates, §1-1 rule), `LibraryService`, `LibraryValues`, re-exports | P1 (header) · P7 |
| `backends.rs` | `Backends` · `AgentBackends` | P1 |
| `path.rs` | `LibPath` · `AssetRef` · `BuiltinId` · `PresetItem` · `ItemOrigin` · `FolderRef` · `PathError` | P1 |
| `format/mod.rs` · `format/read.rs` | Kind sniff, `PresetDoc` · `SettingsDoc` · `Unmodeled` · `parse_preset` · `parse_settings` · `ParseError` · the shared parse options | P3 |
| `format/frontmatter.rs` | `split_front_matter` (the library's front-matter split, M13) · `YamlFrontMatter: FrontMatterParser` · library skill naming (M13) | P3 |
| `format/tree.rs` · `format/write.rs` | The order-keeping YAML tree `Node`; `rewrite_lists` — whole-file rewrite (ADR-0358) | P3 |
| `validate.rs` | `Validator` · `SchemaSet` · settings check · merged-value attribution · skill-format wrapper · `SchemaFetch` · `CliVersionRead` · `refresh_schemas` (the re-check as a change lands with the service, P7) | P4 |
| `store.rs` | `LibraryStore` · `FsStore` · `MemStore` (test) · `ScanListing` · `ContentTag` · `admits` · `first_link` | P5 |
| `catalog.rs` · `excerpt.rs` | `Catalog` · `Item` · `ItemContent` · racy rule · reverse index · instruction excerpt | P5 |
| `refs.rs` | Reachable closure · reverse references (preset side from `Catalog`, agent side from the registry) · counts | P5 |
| `folder.rs` | `BoundedIo` (stat · list · canonicalize · read, one limit, concurrent across keys, one in-flight call per key) · `FolderOps` · `Spawn` · `IoBudget` · `LaunchIo` · `FolderStatus` · `FolderListing` | P6 |
| `gate.rs` | The scan-generation `Mutex` + `Condvar`: tickets, change count, generations, swap-in count, first-scan state, refresh merging | P7 |
| `builtin.rs` | `Builtins` — `engram:/…` → files under the `builtin\` value | P8 |
| `resolve.rs` | `Target` · `Lookup` · `Looked` · `ItemView` · `Resolution` · `Problem` · `ResolvedFolder` · `resolve` · folder-status attach | P8 |
| `launch.rs` | `impl LaunchKitSource for LibraryService` · `merge_instructions` · volatile layout · `sweep_launch_dir` | P9 |

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
pub enum BuiltinId { DefaultPreset, Priming, DefaultSettings }   // engram:/default · engram:/priming · engram:/settings/default — closed set (B23)
pub enum AssetRef { Library(LibPath), BuiltIn(BuiltinId) }
impl AssetRef { pub fn parse(text: &str) -> Result<Self, PathError>; pub fn spelling(&self) -> String; }
pub struct PresetItem {
    pub target: AssetRef,
    pub backend: Option<String /* a word Backends knows */>,
    pub line: u32,
    pub origin: Option<ItemOrigin>,             // where the item was read from; None = a new item (the writer's identity rule — §3-4)
}
pub struct ItemOrigin { pub index: u32 /* position in that list in the file, counting every entry */ }
pub struct FolderRef { pub raw: String, pub normalized: Result<PathBuf, FolderPathError>, pub line: u32, pub origin: Option<ItemOrigin> }
```

`AssetRef::parse` decides by prefix: `engram:/` → built-in (an unknown id = `PathError::UnknownBuiltin`); anything else → `LibPath`. `LibPath` refuses `:`, so the two never mix (§1-2). A tag value that `Backends::for_word` does not know is a broken line (② — `backend-tag-unknown`); a new backend's word is accepted the moment agent registers it (§1-1).

### 3-3a. `backends.rs` (must 10)

```rust
pub trait Backends: Send + Sync {
    /// The settings check of `word`, ASCII case-insensitive; None = no backend of that word has one.
    fn for_word(&self, word: &str) -> Option<&dyn SettingsCheck>;
    /// Every registered settings check, registry order.
    fn all(&self) -> Vec<&dyn SettingsCheck>;
}
pub struct AgentBackends;   // production: agent settings_check_for / settings_checks() — the only caller of those in the crate
```

Threaded through: `parse_preset(text, backends)` (tag words) · `check_settings(doc, set, backends, installed)` · `Validator::refresh_schemas` (`all()`) · `resolve(target, lookup, backends)` (single-preset U5 ranges over `all()`; agent mode uses `for_word(target.backend)`) · `LibraryValues.backends: Arc<dyn Backends>` (service, `check`). The backend word of a profile still comes from agent `backend_word(&profile.command)`; the fake registry in tests answers a fake port for that word.

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
    pub unmodeled: Vec<Unmodeled>,      // list entries the reader could not model — each also a line problem
    pub line_problems: Vec<Reason>,     // per-line breakage that does not stop the read (unknown tag, relative folder …)
}
pub struct Unmodeled { pub list: ListName, pub index: u32 /* position in that list, counting every entry */, pub line: u32 }
pub enum ListName { Presets, Instructions, Skills, Settings, Folders }
pub struct SettingsDoc { pub version: u32, pub description: Option<String>, pub raw: Vec<RawField> }   // raw = every other top-level key, file order
pub struct RawField { pub name: String, pub value: serde_json::Value, pub line: u32 }

pub fn parse_preset(text: &str, backends: &dyn Backends) -> Result<PresetDoc, ParseError>;
pub fn parse_settings(text: &str) -> Result<SettingsDoc, ParseError>;
pub struct ParseError { pub code: &'static str, pub message: String, pub line: Option<u32> }   // → agent `Reason`
```

- **Parse options, one place** (`format::options()`): `strict_booleans: true` (G49) and `non_finite_float_policy: PassThrough` (premise 40 — a `.nan` / `.inf` anywhere, even in a key Engram does not model, does not fail the read). Item shapes: a plain string or `{path, backend}` (M20) — read through an intermediate value enum so a wrong shape is a line problem with its line, not a whole-file failure (B5: `untagged` loses `Spanned` positions — §6-2 L1018).
- An entry that is not a string or a map with a string `path`, or whose tag word is unknown, or whose `path` does not parse, is a **line problem** and is recorded in `unmodeled` with its list and index; it is not in the typed list. The writer passes it through untouched (below).
- Raw fields are kept by name and value only; which field is "this backend's" is decided later by `Backends::for_word(name)` for the agent's word, case-insensitively. A field whose name no backend knows is kept and ignored (a later backend may claim it — §1-1).
- **`folders`**: each item must be a string; `normalized` = `platform::fs::normalize_folder` (§3-10). A relative path or a `LibPath` = `FolderPathError` → broken line ② (§1-2), still in `folders` (it is modeled — a string). A non-string entry = `unmodeled`. Missing key = empty list.

**Front matter** (`frontmatter.rs`):

```rust
pub fn split_front_matter(text: &str) -> Option<FrontMatter<'_>>;   // lines(); first line trim()=="---"; up to next "---"; BOM not stripped
pub struct FrontMatter<'a> { pub yaml: &'a str, pub first_line: u32, pub body: &'a str }
pub struct YamlFrontMatter;                                          // serde-saphyr, format::options()
impl FrontMatterParser for YamlFrontMatter { fn paths(..); fn subagent(..); fn skill(..); fn is_yaml_value(..); }
pub fn library_skill_name(skill_md: Option<&str>, folder_name: &str) -> String;   // front-matter name trimmed+folded, else folder name (M13)
pub fn conflict_key(name: &str) -> String;                                         // lowercase, [^a-z0-9_-] → '-' (M13 · M25)
```

`split_front_matter` is the **library's** front-matter split, used for every backend's skill naming (M13) and the excerpt; its rule was copied from codex's `extract_frontmatter` (parent L278), which is history, not a backend branch. `YamlFrontMatter` is the only real implementation of the agent trait; `validate` passes it to `skill_format` and `folder` passes it to `folder_load_set` / `root_skills` (M63).

**Write** (`tree.rs` · `write.rs` — ADR-0358) — the primitive 9a's `preset.*` and `preset.save` call:

```rust
pub enum Node { Null, Bool(bool), Int(i128), Float(f64), Str(String), Seq(Vec<Node>), Map(Vec<(Node, Node)>) }
impl<'de> Deserialize<'de> for Node;   // a visitor that keeps map order and non-string keys (format::options())
impl Serialize for Node;               // maps in stored order; Int → i64 or u64 by range
impl Node { pub fn same(&self, other: &Node) -> bool; }   // structural equality; Float compared by to_bits() (must 4 (d))

pub struct PresetLists {
    pub presets: Vec<PresetItem>, pub instructions: Vec<PresetItem>, pub skills: Vec<PresetItem>, pub settings: Vec<PresetItem>,
    pub folders: Vec<FolderItem>,
    pub drop_unmodeled: Vec<(ListName, u32 /* index */)>,   // unmodeled entries the caller removes on purpose; every other one is kept
}
pub struct FolderItem { pub path: String, pub origin: Option<ItemOrigin> }
pub fn rewrite_lists(text: &str, lists: &PresetLists) -> Result<String, WriteError>;
pub enum WriteError { Parse(ParseError), NotAPreset, MultipleDocuments, NumberOutOfRange { path: String }, RoundTrip { path: String /* first differing node */ } }
```

`rewrite_lists`:
1. Strip a leading U+FEFF and remember it; remember the line ending (CRLF if the first line break is `\r\n`, else LF; mixed endings are written as the first one — the file is rewritten anyway).
2. Parse the text into `Node` (`format::options()`). More than one document ⇒ `MultipleDocuments` (code `preset-multi-document`, write nothing). Not a top-level map with `kind: preset` ⇒ `NotAPreset`.
3. **Number guard** (must 4 (c) · premise 39): any `Float` node whose value is integral and whose magnitude is ≥ 2^63 ⇒ `NumberOutOfRange` (code `preset-write-number-range`, `args` = the node path), nothing written. The tree cannot tell a 21-digit integer literal (already rounded by the reader) from a written float such as `1e20`, so both are refused rather than kept as text: keeping the literal would need the source text of the scalar, which serde-saphyr does not hand to a visitor. The refusal names the key; the user edits that value by hand. Other floats keep their `f64` value (digits beyond `f64` precision in a float literal are formatting, ADR-0358).
4. Replace only the five list nodes (`presets` · `instructions` · `skills` · `settings` · `folders`) in place — an absent key is appended after the last present one; an emptied list is written as `[]`. **Every other key, known or not, stays as the parsed node in its original order** (ADR-0358 「영향」). Each new list is built entry by entry:
   - **Item identity** (must 4 (a)): an item with `origin = Some(i)` starts from the original entry node `i` of that list; an item with `origin = None` takes the first original **modeled** entry of the same list with the same `target` spelling that no other item has taken (this covers `preset.save`, whose lists come from a client without origins), else it is new. Identity never depends on the tag or the position, so a reorder and a tag change keep the item's node.
   - **Writing an item from its node**: a map node keeps every key it had, in order (unknown keys inside an item map survive); only the `backend` value is set, added (after `path`) or removed. A string node with a tag becomes `{path, backend}`. A map node that loses its tag and has no other key than `path` becomes a string again; with other keys it stays a map. A new item = a string, or `{path, backend}` with a tag.
   - **Unmodeled entries** (must 4 (b)): every original entry in `unmodeled` that is not in `drop_unmodeled` is copied as its original node, anchored after the nearest original entry before it that is still in the new list (or at the head of the list when none is). A list key that only holds unmodeled entries keeps them.
5. Serialize with `to_string_with_options(&node, ser_options! { indent_step: 2 })` — `yaml_12` stays `false`, so strings that a YAML 1.1 reader would take as booleans or numbers are quoted.
6. **Round-trip check** — re-parse the output into `Node` and compare with the tree from step 4 by `Node::same`; any difference ⇒ `RoundTrip` (code `preset-write-mismatch`), nothing written.
7. Re-apply the line ending (LF → CRLF if remembered) and the BOM.

**Why the round-trip check stays**: ADR-0333's gate existed to catch `yaml-edit`'s broken outputs and is gone with it. A different risk remains: a serializer that writes a scalar a reader takes back as another value (a numeric-looking string, a float's text, an ambiguous plain scalar) would silently change a user's file — the one failure ADR-0358 does not accept. The check costs one parse of a small file. It cannot see a loss that happened at read time (premise 39), which is why step 3 exists.

What is lost on a rewrite, by decision (ADR-0358): comments, blank lines, quoting style, flow vs block style, key spacing, anchors/aliases (expanded — values equal), merge keys (`<<` expanded — values equal), explicit tags (`!!str` etc. — the tagged value stays), the sign of a NaN. Indentation becomes 2 spaces.

### 3-5. `store`, `catalog`, `refs`

```rust
pub trait LibraryStore: Send + Sync {
    fn scan(&self, root: &Path, under: Option<&LibPath>, seq: &dyn Fn() -> u64) -> io::Result<ScanListing>;
    fn first_link(&self, root: &Path, rel: &LibPath) -> io::Result<Option<LibPath>>;   // first component below the root that is a link (U1)
    fn read(&self, path: &Path, cap: u64) -> io::Result<Vec<u8>>;
    fn tag(&self, path: &Path) -> io::Result<ContentTag>;
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;   // parent missing → NotFound, nothing created
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy(&self, from: &Path, to: &Path) -> io::Result<()>;             // base copy_atomic
    fn trash(&self, root: &Path, path: &Path, bundle: &TrashBundle) -> io::Result<PathBuf>;   // rename into <root>\.trash\<bundle>\<same rel path>
    fn exists(&self, path: &Path) -> bool;
}
pub struct ScanListing {
    pub files: Vec<ScanEntry>, pub folders: Vec<LibPath>,
    pub listed_at: HashMap<LibPath /* folder; root = "" */, u64>,   // seq drawn just before that folder was listed (must 6)
    pub skipped_links: Vec<LibPath>, pub errors: Vec<(LibPath /* folder whose walk failed */, String)>,
}
pub struct ScanEntry { pub rel: LibPath, pub tag: ContentTag }
#[derive(PartialEq, Eq)] pub struct ContentTag { pub modified: SystemTime, pub size: u64, pub identity: Option<FileIdentity> }
pub struct TrashBundle(pub String);   // placeholder — the naming is 9b's (§6-5 1); step 7 only passes it through
pub fn admits(rel: &LibPath) -> bool; // the name rule every way into Catalog shares: no component starting with '.', no base temp name
```

`FsStore` = walkdir + std + base `write_atomic` / `copy_atomic` with `OsHooks { retry: platform retry_busy, sync_dir: platform sync_dir }`.

- **Link policy (U1 (a)) on every path** (must 5): only the **configured library root** may be a link — `follow_root_links(true)` for a full scan only. A partial scan first runs `first_link(root, under)`: if `under` or any folder between the root and it is a link, nothing is walked, `under` goes into `skipped_links`, and its range is published empty (the same result a full scan gives, which never enters that link). Inside a walk `follow_links(false)`; an entry that is a link (file or directory — `StdLaunchFs::is_link` semantics) is not entered and not listed but recorded in `skipped_links`, which `refresh` returns. `first_link` checks each component below the root with `symlink_metadata` (cheap, local); `MemStore` models links. It is the **one** component test used by the partial scan, the `list` safety net, `check`'s overlay reads and every late merge (§6).
- `filter_entry` drops what `admits` refuses (names starting with `.`, `base::file::is_temp_name` names). The same `admits` + `first_link` + kind sniff gate the safety net and late merges, so a late merge admits only paths the scan would admit.
- **Per-folder listing sequence** (must 6 · ADR-0324 decision 5): walkdir yields a folder entry before it reads that folder; the scan draws `seq()` at that moment and records it in `listed_at`. A file inside the scan range that the listing does not contain counts as read 「absent」 at `listed_at[its parent]` (or, when the parent itself is gone, at the `listed_at` of its nearest listed ancestor). One number per scan (draft 2's `listed_seq`) could not do this: a deep folder listed late would carry an early number and lose to a check that read the file before it was deleted.
- **A failed folder walk keeps what was known** (recommended): a folder in `errors` (permission, a transient IO error) keeps its previous items unchanged in the swap-in (they are outside what the scan could see), so a transient error never turns them into a false 「찾을 수 없음」; `refresh` reports the error.
- `trash` is a same-root rename (B8 · G31).

```rust
pub struct Catalog { items: HashMap<PathKey, Item>, folders: HashMap<PathKey, LibPath>, used_by: HashMap<PathKey, BTreeSet<PathKey>>, racy: HashSet<PathKey>, scanned_at: SystemTime }
pub struct Item { pub path: LibPath, pub kind: ItemKind, pub tag: ContentTag, pub read_seq: u64, pub content: ItemContent, pub schema_gen: u64 }
pub enum ItemKind { Instruction, Skill, SkillFile, Preset, Settings, Broken { claimed: Option<Sniff> } }
pub enum ItemContent {
    Instruction { excerpt: String },
    Skill { name: String, description: String },             // library_skill_name; skill_format runs per agent mode in resolve
    SkillFile,
    Preset { doc: PresetDoc },
    Settings { doc: SettingsDoc, verdict: SettingsVerdict },
    Broken { reason: Reason },
}
impl Catalog {
    pub fn build(listing: ScanListing, range: Option<&LibPath>, prev: Option<&Catalog>, read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
                 v: &Validator, backends: &dyn Backends, now: SystemTime) -> Catalog;
    pub fn get(&self, key: &PathKey) -> Option<&Item>;
    pub fn children(&self, folder: Option<&LibPath>) -> Vec<&Item>;
    pub fn merge_item(&mut self, item: Item, may_insert: bool);   // the late merge's write (the rule lives in the service)
}
```

- Kind decision = §1-2 table. A file is re-read only when its tag differs (equality, not "newer") or it carries the racy mark (mtime ≥ scan time − 2 s). Unsupported identity = `None`, not a failure (premise 20).
- `used_by` = preset → items, direct + indirect (Δ3). Agent-side references are never stored (M5).
- Excerpt (`excerpt.rs`, R15 · §6-2 L1003) — pulldown-cmark event walk that skips the `MetadataBlock` subtree, code blocks, tables (`ENABLE_TABLES` on so they are recognised) and HTML; the first paragraph's text, whitespace collapsed, capped (B16).
- `refs.rs`: `closure(catalog, preset) -> Vec<AssetRef>` (excludes `engram:/` and folder entries — ADR-0341) · `referrers_of(catalog, registry, item) -> (Vec<LibPath>, Vec<AgentId>)` (presets from `Catalog.used_by`, agents from `ProfileRegistry::attachments()` with `same_path`) · `reference_count(…)` (the `list` row — premise 46). One walk, reused by `list`, 9a · 9b · T-73.

### 3-6. `validate`

```rust
pub struct Validator { schemas: RwLock<Arc<SchemaSet>> }   // a reader clones the Arc and releases at once; a refresh swaps the Arc (a leaf lock — §6)
impl Validator { pub fn current(&self) -> Arc<SchemaSet>; }
pub struct SchemaSet { pub generation: u64, per_backend: HashMap<String /* backend word */, HeldSchema> }
pub struct HeldSchema { pub source: SchemaOrigin /* Shipped | Cached */, pub cli_version: Option<String>, compiled: Arc<jsonschema::Validator> }

pub fn check_settings(doc: &SettingsDoc, set: &SchemaSet, backends: &dyn Backends, installed: &dyn Fn(&str) -> Option<String>) -> SettingsVerdict;   // every raw field whose name a backend knows
pub fn check_merged(backend: &dyn SettingsCheck, layers: &[(AssetRef, &Value)], set: &SchemaSet) -> Vec<(AssetRef /* the layer that brought the value */, Reason)>;
pub fn check_skill(backend: &dyn SettingsCheck, mode: AgentOutputFormat, file_name: &str, folder: &str, text: &str) -> SkillFormat;  // passes YamlFrontMatter
pub struct SettingsVerdict { pub problems: Vec<(String /* field */, Reason)>, pub notices: Vec<(String /* field */, Notice)> }

pub trait SchemaFetch: Send + Sync { fn fetch(&self, url: &str, etag: Option<&str>) -> Result<Fetched, String>; }
pub enum Fetched { NotModified, Body { bytes: Vec<u8>, etag: Option<String> } }
pub trait CliVersionRead: Send + Sync { fn read(&self, probe: &VersionProbe) -> Option<String /* stdout */>; }
impl Validator { pub fn refresh_schemas(&self, backends: &dyn Backends, fetch: &dyn SchemaFetch, read: &dyn CliVersionRead, cache_dir: &Path) -> bool /* swapped */; }
```

- For each raw field whose name `Backends::for_word` knows: `raw_to_json` (port) → schema `iter_errors` → type · range error = ② with the line of that field; unknown key per `unknown_keys()`: `Ignore` = silent at any depth (incl. `additionalProperties:false` violations) · `Warn` = the notice `settings-unknown-key` (U4 · B8); no schema held = notice `settings-schema-unavailable`; **under `Warn` only**, a schema older than the installed CLI turns the unknown-key notices into the one notice `settings-schema-outdated` (P10-ADD · U3) — under `Ignore` unknown keys are silent anyway, so that notice is never emitted.
- **Which field blocks** (recommended): a launch blocks only on the agent's own backend field (②). A schema or `-c` error in another backend's field of the same asset never blocks that agent; it shows on the asset's own row (`list` state, `preset.check` in 9a) and blocks agents of that backend. Same "only that backend's raw field is loaded" rule as the parent (§1-3).
- Each value also passes `command_line_args` (port — a backend whose settings reach the command line refuses unsafe lines ②; others answer `Ok(vec![])`).
- A merged value (`merge` → `Some`) is checked as a whole; `check_merged` takes the layers as `(AssetRef, value)` — the built-in settings layer (`engram:/settings/default`) is one of them — and uses `merge_origins` (§3-9) so an error lands on the asset row that brought the value (M14). The merge is never re-derived here (step-5 hand-over).
- jsonschema options are explicit: draft auto-detected; format validation on for known formats, unknown formats ignored, stated in code (§1-12).
- `refresh_schemas`: for every port of `backends.all()`, read the CLI version through `CliVersionRead` (outside every lock), compare with the held copy per `schema_source(version).refetch`, fetch through `SchemaFetch` (outside every lock), write the cache file atomically, compile, swap the `Arc`. The service then draws a ticket and re-checks every settings item as a `Catalog` change (P7 · §6 · ADR-0332 「영향」). Failure keeps what is held.

### 3-7. `resolve` and `builtin`

```rust
pub struct Target { pub backend: Option<String>, pub mode: Option<AgentOutputFormat>, pub default_preset: bool, pub presets: Vec<AssetRef>, pub root: RootState }
pub enum RootState { Unset, Unavailable(String /* missing · not a folder · "timeout" */), Ok }
pub trait Lookup { fn item(&self, r: &AssetRef) -> Looked<'_>; }
pub enum Looked<'a> { Present(ItemView<'a>), Missing, Unreadable(Reason) /* incl. `path-through-link` */ }
pub struct ItemView<'a> { pub at: &'a AssetRef, pub kind: &'a ItemKind, pub content: &'a ItemContent, pub file: Option<&'a Path> /* disk path; None for an in-memory built-in */ }
pub fn resolve(target: &Target, lookup: &dyn Lookup, backends: &dyn Backends) -> Resolution;
pub struct Resolution {
    pub instructions: Vec<(String /* source path */, PathBuf)>,
    pub skills: Vec<(String /* name */, PathBuf, Vec<String> /* via */)>,
    pub raw_settings: Vec<ResolvedSettings>,
    pub folders: Vec<ResolvedFolder>,
    pub include_priming: bool,
    pub problems: Vec<Problem>,
    pub notices: Vec<Notice>,
}
pub struct ResolvedSettings { pub asset: AssetRef /* M14 attribution */, pub via: Vec<String>, pub value: Value }
pub struct ResolvedFolder { pub path: PathBuf /* normalized */, pub key: PathKey, pub via: Vec<String>, pub status: Option<FolderStatus> /* phase 2 */ }
pub struct Problem { pub kind: ProblemKind, pub item: String /* library path · engram:/… · folder:<path> */, pub reason: Reason, pub via: Vec<String> }
pub enum ProblemKind { Missing /* ① */, Broken /* ② */, Conflict /* ③ */, LibraryUnavailable /* ④ */ }
impl Resolution {
    pub fn folder_paths(&self) -> Vec<PathBuf>;                                       // phase 1: what to stat
    pub fn attach_folder_status(&mut self, st: &HashMap<PathKey, FolderStatus>);      // phase 2: ① lines
    pub fn blocked(&self) -> Option<LaunchBlocked>;
}
```

- **Pure** — no disk. Lookup is `Catalog` + the overlay of bytes the launch check read (§6), or `MemStore` in tests. Folder status arrives through phase 2 (the injected seam — ADR-0341 · §1-3).
- Expansion order, dedup at first place, cycle, per-preset tag order (G51), conflicts on loaded skills only, notices, folder lines, block ④ (a library preset attached while `root` ≠ `Ok`). What is loaded and which skill names are errors come only from the port (`loads` · `accepts_skill_name`). **An item tagged for another backend, or a settings asset without this backend's field, is skipped silently** — no notice (D2).
- **Single-preset mode** (no agent — `backend: None`): everything is shown; a same-name skill pair is a conflict only if some backend would load both (U5 (a)): common × common, common × tagged, same tag × same tag; two different tags never conflict. "Some backend" ranges over `backends.all()`, so a new backend is covered without change.
- `builtin.rs`: `Builtins::new(dir: Option<PathBuf>)`; `engram:/default` = `<dir>/presets/default.yaml` (a preset in library shape) etc. — file names B14. Missing dir → the built-in preset expands to nothing, `include_priming = false`, a log warning (M7 · B15). A library preset that lists `engram:/…` in `presets` = broken line (Δ1). The id set is closed (B23).

### 3-8. `folder` and `launch`

```rust
pub struct BoundedIo { limit: Duration /* 5 s — D1 */, list_cap: usize /* 10,000 — U2 */, inflight: Mutex<HashMap<IoKey, Arc<Pending>>>, ops: Arc<dyn FolderOps>, spawn: Arc<dyn Spawn> }
pub enum IoKey { Stat(PathKey), List(PathKey), Canonical(PathKey), Read(PathKey) }
pub trait FolderOps: Send + Sync {                       // production = std + dunce; tests = gate-controlled fakes
    fn stat(&self, p: &Path) -> FolderStatus;
    fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus>;   // read-only, links not entered, ≥ six levels
    fn canonical(&self, p: &Path) -> io::Result<PathBuf>;                         // dunce::canonicalize — the manager's (manager.rs:1566)
    fn read(&self, p: &Path, cap: u64) -> io::Result<String>;
}
pub trait Spawn: Send + Sync { fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()>; }   // production = std::thread::Builder
pub enum FolderStatus { Ok, Missing, NotADirectory, Unreadable(String /* OS error or "timeout" */) }
pub struct FolderListing { pub entries: Vec<agent::backend::settings_check::FolderEntry>, pub truncated: bool }

/// IO time left for one launch or one read computation — charged only by time spent waiting on bounded IO (must 3).
#[derive(Clone)] pub struct IoBudget { left: Arc<Mutex<Duration>> }
/// One launch's (or one read computation's) IO context: its budget and every answer it got (D1 · must 2).
pub struct LaunchIo { pub budget: IoBudget, answers: Mutex<HashMap<IoKey, IoAnswer>> }
impl BoundedIo {
    pub fn begin(&self) -> Arc<LaunchIo>;                                                  // a fresh 5 s budget and an empty answer set
    pub fn stat_all(&self, io: &LaunchIo, paths: &[PathBuf]) -> HashMap<PathKey, FolderStatus>;
    pub fn list_all(&self, io: &LaunchIo, paths: &[PathBuf]) -> HashMap<PathKey, Result<FolderListing, FolderStatus>>;
    pub fn canonical_all(&self, io: &LaunchIo, paths: &[PathBuf]) -> HashMap<PathKey, Result<PathBuf, FolderStatus>>;
    pub fn read_one(&self, io: &LaunchIo, path: &Path, fresh: bool) -> io::Result<String>;   // the port's `read` closure; `fresh` = do not reuse an answer (prepare)
}
```

- **One limit, 5 s** (D1 — replaces U2's 10 s), for every bounded folder operation: stat, listing, canonicalize, file read. No separate values. `LibraryValues.io_limit` holds it so tests shorten it.
- **One budget per launch** (D1): a launch is `check_before_create` → `prelaunch` (up to `PRELAUNCH_ATTEMPTS` checks) → `prepare` — premise 42. All of them run in one `LaunchScope` (§3-9) holding one `LaunchIo`, so they share **one 5 s budget and one answer set**: a folder answered earlier in the launch is not asked again in the same launch (a second check after an attachment change asks only the new keys, on the time left). A read computation outside a launch — `resolve_preset`, `resolve_agent` (`agent.presets`, `preset.get`), tree ⛔ over many agents — calls `begin()` once and gets one 5 s budget for the whole computation. So the user-visible worst case of a blocked launch is ≈ 5 s of folder IO, inside the 7 s bus slot (premise 41).
- **The budget is charged only by IO waiting** (must 3): a batch waits until `now + left`, then subtracts the time it actually waited. Time spent waiting for the library lock, the gate, parsing or any other work is never charged, so a read-lock stall (ADR-0324 M47 — accepted) cannot use up the budget and turn a healthy folder into ①. A batch that starts with nothing left answers every key it has not already got as `Unreadable("timeout")` without waiting.
- **Concurrent per batch**: a call takes every distinct key it needs at once, starts one worker per key that has no call in flight, and waits on all of them against the same deadline. N unreachable folders therefore cost one budget in total, not N × 5 s.
- **Entry lifetime** (must 2): the in-flight map holds only calls that are running. The worker removes its entry (only if the map still holds the same `Arc` — `Arc::ptr_eq`) when it completes; joiners hold their own `Arc<Pending>` and read the answer from it. A completed answer lives only in the `LaunchIo` that asked for it and dies with it, so a folder that answered `Missing` is asked again by the next launch (a folder created in between is seen). The worker's job owns a **completion guard** whose `Drop` fills an empty answer (`Unreadable("worker panicked")`), wakes every waiter and removes the entry — it runs on a panic and also when `Spawn::spawn` fails and drops the job unrun (the answer is then `Unreadable("spawn failed: …")`). **A call that has already run longer than one full limit** is not joined: a new caller gets `Unreadable("timeout")` for that key at once (the worker stays abandoned until it returns, then removes its entry; the next caller after that starts fresh). Abandoned workers are bounded by the distinct keys that hang.
- **Bounded file reads** (must 1): the `read` closure handed to `folder_load_set` / `root_skills` is `|p| io.read_one(&launch_io, p, false)`; `prepare`'s read of a folder instruction file is `read_one(…, true)`. Each read is one key on the same budget and per-file cap (a named constant; over it = an error the port records as `unsure`). The port calls `read` one file at a time, and each call waits only for the time left, so a held file read still ends the whole check inside the budget. **What a read timeout becomes**: the closure returns `ErrorKind::TimedOut`, the port files that row as `unsure` (its rule — premise 43), **and** asset-library, which owns the closure, marks that folder entry `Unreadable("timeout")` ⇒ ① for the launch. Over the budget = unreadable = ①, whichever operation hit it. A read error that is not a timeout (permission, file too large) stays the port's `unsure` row — a preview mark, no block, as today.
- **Over the budget = unreadable = block ①** for the whole launch, for a **folder entry** (ADR-0341 decision 6, user re-confirmed 2026-11-11 「그냥 하나 막혀도 전체 막는게 좋겠지? 구지 무리하게 실행하는것보다는」). The **library root**: a root stat that does not answer = `RootState::Unavailable("timeout")` ⇒ ④ when a library preset is attached. A **native skill root** (not named by any preset — only the input of the same-name notice): a timeout leaves that root out of the comparison and is logged; no block (D1 detail, main — §10-2 B24).
- **Listing cap 10,000 entries**: past it the listing stops and is marked `truncated`; the preview says it is partial, nothing blocks (U2), and the launch reply carries the notice `folder-listing-partial` (`args` = `{folder}`) so the user knows the carried rows and notices were computed on a partial listing (recommended — the port cannot name the entries it needs before it sees a listing, so reading them first is not possible without a new port answer).
- **Folder IO never runs under the library lock** (§6); a test fake op asserts `try_write` on the library lock succeeds while it runs.

```rust
impl LaunchKitSource for LibraryService {
    fn begin_launch(&self) -> LaunchScope;                                        // LaunchScope::new(self.io.begin())
    fn check_in(&self, scope: &LaunchScope, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked>;
    fn prepare_in(&self, scope: &LaunchScope, plan: LaunchPlan, id: AgentId, epoch: u32) -> Result<LaunchKit, PrepareError>;
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked>;   // = check_in(&self.begin_launch(), profile)
    fn prepare(&self, plan: LaunchPlan, id: AgentId, epoch: u32) -> Result<LaunchKit, PrepareError>;   // = prepare_in(&self.begin_launch(), …)
    fn release(&self, id: AgentId, epoch: u32);
}
pub fn merge_instructions(priming: Option<&str>, sources: &[(&str /* source path */, &str /* text */)]) -> String;   // the one merge — separators included
pub fn sweep_launch_dir(launch_root: &Path) -> SweepReport;   // boot — step 8 calls it before plugging the port
```

A scope that is not asset-library's (e.g. an empty one) is treated as a fresh one — `check_in` never fails on the scope.

**`check_in`** (one read-lock take — §5-2 4 (d)):
1. `word = backend_word(&profile.command)`; `checks = backends.for_word(&word)`; `None` (a backend with no settings check, e.g. Shell) ⇒ the Noop plan, `real: false` (B13). `mode = output_format`; `loads = checks.loads(mode)`; `codes = checks.notice_codes(mode)`.
2. Note the three numbers (change generation · root generation · swap-in count) from the gate.
3. **Outside every lock — IO batch 1**: stat the library root (bounded). Read the attached preset files from disk and, as they parse, the files they reach (presets · settings · `SKILL.md` · instruction existence) — the fresh-bytes overlay, with a SHA-256 digest of each file's bytes; each path first passes `admits` and `first_link` (U1 — a path through a link inside the library is `Looked::Unreadable(path-through-link)` ②, never read). Read the built-in priming text. Library files are read with the store directly (local by assumption — R4).
4. **Read lock once**: `resolve` over `Catalog` + overlay with `backends`; settings verdicts for the attached settings; release.
5. **Outside every lock — IO batches 2–4, all on the scope's budget**: (2) phase-2 stats of the folder entries, stats of the native skill roots (`native_skill_roots(places, mode)`), and the canonical working folder (`canonical_all` on `profile.cwd` — on an error or a timeout the manager's fallback applies: `profile.cwd` as is, the same value `manager.rs:1566` would use; `materialize` re-applies the working-folder rule, so a later different canonical value is caught there — ADR-0353); (3) listings — of each folder entry only when `checks.folder_listing_at_launch(mode)` is true, and of each existing native root; (4) the port's file reads through `read_one`, then `canonical_all` on the **folder roots** only (each folder entry and native root — a listing never enters a link, so the canonical path of a `SKILL.md` = canonical(root) + its relative path; recommended). From these: `folder_load_set(…, YamlFrontMatter, mode)` per listed folder — its **`carried`** rows give the instruction file and the extra skill roots the plan carries (never `loads`, which is the preview — must 9), its `omitted` gives a `codes.folder_omitted` notice; when native roots exist, the same-name comparison (library skill names + folder `skill_names` + `root_skills` per native root, each `SKILL.md` kept once per canonical path — step-6 hand-over (a)) gives one `codes.same_name` notice per name; `command_line_args` + `command_line_fits` with `cwd` = the canonical working folder and `instructions` = `merge_instructions(…)` over what this mode loads; `working_folder_blocks(canonical cwd, mode)` (hand-over (d)); `folder_entry_blocks` per folder; a timed-out folder entry ⇒ `Unreadable("timeout")` ⇒ ①; a truncated listing ⇒ `folder-listing-partial`.
6. **Short write lock, only if free** (recommended): `try_write`; when it succeeds, the conditional late merge of the overlay (§6); when contended, the merge is skipped (it is a cache refresh — `check` used its overlay either way, and must not wait behind a writer inside the bus slot).
7. Problems ⇒ `Err(LaunchBlocked)` in the §1-3 8 order; else `Ok(LaunchPlan { real: true, … })`. When `loads.instructions == BuiltinPrimingOnly`, the plan carries no instruction sources; when `loads.skills == false`, the plan carries no skills; the omitted ones of both go into one `codes.omitted` notice (hand-over (b) · recommended). Notices carry identity-only `args` (premise 12). The plan's `stamp` records the SHA-256 of every file whose bytes `check` used (B7 · must 7).

**`prepare_in`**: no library lock (must 7). Create `<launch_root>\<id>-<epoch>\`; re-read each source in the plan's `stamp` (library files with the store; a folder instruction file with `read_one(…, fresh: true)` on the scope's budget) and compare the SHA-256 with the stamp — gone, unreadable (incl. a timeout) or different bytes ⇒ `Stale` (byte-exact: an in-place edit of the same length with the same mtime is caught; ADR-0336 sha2); write the merged instructions file with `merge_instructions` from the bytes just read (priming first when `include_priming`, then the plan's sources in order — §1-5 L318); `instructions_hash` = B9; `last_applied_instructions_hash = None` (hand-over (e)); `include_priming = true` ⇒ always an `instructions_file` (hand-over (e)). Other failures ⇒ `Failed`.

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

/// Whether a launch in `mode` needs a listing of each folder entry — to carry rows from it or to name what it omits.
/// `false` = the backend reads the folder itself at launch (only the preview lists it).
fn folder_listing_at_launch(&self, mode: AgentOutputFormat) -> bool;

// FolderLoadSet gains:
/// The rows the launch itself carries from this folder in this mode — an `Instructions` row joins the merged
/// instructions, a `SkillRoot` row is passed as an extra skill root. Empty when the backend loads the folder natively
/// (it gets the folder path) or loads nothing from it in this mode. A subset of `loads`.
pub carried: Vec<FolderLoad>,

// launch.rs — LaunchKitSource gains three default methods (D1)
/// One launch's shared context — opaque to the manager. A source keeps its per-launch state in it.
#[derive(Clone, Default)] pub struct LaunchScope(Option<Arc<dyn Any + Send + Sync>>);
impl LaunchScope { pub fn new<T: Any + Send + Sync>(v: Arc<T>) -> Self; pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>>; }
fn begin_launch(&self) -> LaunchScope { LaunchScope::default() }
fn check_in(&self, _scope: &LaunchScope, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> { self.check(profile) }
fn prepare_in(&self, _scope: &LaunchScope, plan: LaunchPlan, id: AgentId, epoch: u32) -> Result<LaunchKit, PrepareError> { self.prepare(plan, id, epoch) }
```

- claude `merge_origins` = a walk of the same `merge_two` rule (`backend/claude/settings.rs:439`) that records origins — a test runs both on the same tables. codex = `None`.
- codex `working_folder_blocks` = `Terminal` → `cwd_reaches_codex_unchanged` moved into one `pub(super)` helper that `materialize` also calls (ADR-0353 — one rule), code `codex-working-folder-unsafe` owned by the codex backend · `StreamJson` → `None`; claude → `None`.
- codex `notice_codes` = `Terminal` → `{omitted: CODEX_TERMINAL_OMITTED, folder_omitted: CODEX_TERMINAL_FOLDER_OMITTED, same_name: None}` · `StreamJson` → `{same_name: CODEX_SAME_NAME_SKILLS, ..None}`; claude → all `None`. A new backend answers its own (B20).
- `folder_listing_at_launch`: claude → `false` (both modes — `--add-dir`, parent L314 · L319); codex → `true` (JSON carries rows; terminal names omissions). `carried`: claude → empty; codex JSON → the `Instructions` row and the `SkillRoot` rows of `loads`; codex terminal → empty (its `omitted` holds the JSON set). The 9 `FolderLoadSet` literals in agent are updated (premise 45).
- `LaunchPlan` gains `stamp: Option<String>` (opaque — `check` writes, `prepare` reads; Noop `None`) — B7; **6 literals** (premise 45).
- `LaunchScope` + the three default methods: `NoopLaunchKits` keeps the defaults (argv goldens unchanged). Until step 8 threads a scope, each `check` / `prepare` call makes its own (the crate has no caller yet — §1). Step 8: the manager calls `begin_launch()` once per launch and passes the scope to every `check_in` / `prepare_in` of it (§12).
- `launch.rs`: the generic notice codes of U3 · U4 · B22 (`settings-schema-unavailable` · `settings-schema-outdated` · `settings-unknown-key` · `folder-listing-partial`) get their own sentence in `own_wording`; they take the names from `args`, so no backend is named in the code.

### 3-10. base and platform additions

- **base** `file::is_temp_name(file_name: &str) -> bool` — true for `<anything>.tmp<pid>.<n>` with both numbers exact decimals as `temp_path` writes them; `<x>.corrupt.tmp<pid>.<n>` is true by the same shape. Table test per §4 L785. `temp_owner` is rewritten on top of it so the two cannot drift.
- **platform** `fs::normalize_folder(path: &Path) -> Result<PathBuf, NotAbsolute>` beside `path_key` (§1-2): strip `\\?\` / `\\?\UNC\` → absolute check (relative refused) → `\` separators → lexical `.`/`..` collapse → no trailing separator except a drive or UNC root. No disk access. Non-Windows: `/` separators, same lexical steps. Its doc cross-links `path_key`.

### 3-11. The service

```rust
pub struct LibraryValues {
    pub registry: Arc<ProfileRegistry>,
    pub backends: Arc<dyn Backends>,            // AgentBackends in production (§3-3a)
    pub root: Option<PathBuf>,                  // 9a replaces with LibraryConfig
    pub builtin_dir: Option<PathBuf>,
    pub launch_root: PathBuf,                   // daemon\run\launch
    pub schema_cache_dir: PathBuf,              // daemon\cache\schemas
    pub shipped_schema_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,                  // NativePlaces.home for native_skill_roots
    pub env: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,   // daemon env (profile env applied on top per agent)
    pub io_limit: Duration,                     // 5 s (D1) — a value so tests can shorten it
}
struct State { catalog: Arc<Catalog>, root: RootState, schema_gen: u64 }   // behind the one RwLock; a scan builds from a cloned Arc (no guard held through the build)
impl LibraryService {
    pub fn new(values: LibraryValues, store: Arc<dyn LibraryStore>, io: BoundedIo) -> Self;
    pub fn list(&self, folder: Option<&LibPath>) -> ListAnswer;               // first-scan join; read lock; safety net for one unknown path
    pub fn refresh(&self, folder: Option<&LibPath>) -> RefreshReport;          // ticketed scan (or join per ⑤)
    pub fn resolve_preset(&self, preset: &LibPath) -> Resolution;              // first-scan join; folder stats outside the lock, one budget
    pub fn resolve_agent(&self, id: AgentId) -> Option<Resolution>;
}
pub struct ListAnswer { pub folder: Option<LibPath>, pub rows: Vec<ListRow>, pub builtins: Vec<ListRow> }
pub struct ListRow { pub path: String, pub kind: ItemKind, pub name: String, pub state: Option<Reason>, pub references: usize, pub description: String }   // parent L368
pub struct RefreshReport { pub changed: usize, pub problems: Vec<(LibPath, Reason)>, pub no_kind: Vec<LibPath>, pub skipped_links: Vec<LibPath>, pub errors: Vec<(LibPath, String)> }
```

## 4. Data shapes on disk

Unchanged from the parent §1-2 except the writer (ADR-0358).

- **Kinds** — §1-2 table. `.yaml`/`.yml` with top-level `kind: preset` / `kind: settings`; broken or unknown `kind` = 「오류」 row; no `kind` = not ours (not listed; `refresh` reports 「kind 없음」 for re-read ones); `SKILL.md` directly in a folder = skill (name per the OS rule — `same_path`); any other `.md` = instruction; names starting with `.`, base temp names and anything reached through a link inside the library (U1 — on every path, §3-5) are not scanned, listed or merged.
- **Preset** — `kind` · `version` · optional `description` · `presets` · `instructions` · `skills` · `settings` · `folders` · any other key (kept by edits). **Two item shapes** (M20): a string (common) or `{path, backend}` — only in `instructions` and `skills`; `backend` = any word the backend registry knows; a tag in `presets` / `settings` = broken line; other keys inside an item map are kept by edits. `folders` = plain absolute strings only.
- **Settings** — `kind` · `version` · optional `description` · raw fields named by backend word (case ignored; today `claude` = the settings object, `codex` = a list of `key=value` strings — the shape is the port's knowledge).
- **Folders read/write** — read into `FolderRef { raw, normalized }`; a read never rewrites the file; `resolve` keys by `path_key(normalized)`; writes (9a) store the normalized form.
- **Edits rewrite the whole file** (ADR-0358 · §3-4 write): serde-saphyr serializer, 2-space indent, YAML 1.1-safe quoting, the original line ending and BOM kept; comments, blank lines, quoting and flow style are not kept; unmodeled keys (top level and inside items), their order, and list entries the reader could not model are kept; a multi-document file and a file holding an integer beyond 64 bits are refused.
- **Volatile layout** (written by `prepare` only): `<launch_root>\<id>-<epoch>\` + the merged instructions file (name B14); backend `materialize` adds the rest inside it.

## 5. Hand-over items → where they land

| Item | Source | Lands in | Unit |
|---|---|---|---|
| `format.rs` reads and writes `folders` | ADR-0341 in 7 | §3-4 (`PresetDoc.folders`, `PresetLists.folders`). The golden pins values, unknown-key survival (top level and in items) and unmodeled entries | P3 |
| `resolve.rs` collects folder entries (order · dedup over normalized paths · stat through the injected seam) | ADR-0341 in 7 | §3-7 two-phase `Resolution` | P8 |
| `folder.rs` (stat with timeout · bounded read-only listing) | ADR-0341 in 7 | §3-8 `BoundedIo` (D1 · U2) | P6 |
| The path normalizer | ADR-0341 in 7 | §3-10 platform `fs::normalize_folder` | P1 |
| Folder lines in the problem list and block ① (every backend and mode) | ADR-0341 in 7 | `attach_folder_status` → `Missing { item: "folder:<path>", code: folder-not-found }` | P8 · P9 |
| Folder omission notices (codex terminal today) | ADR-0341 in 7 | `check_in` step 5 → `codes.folder_omitted` from `FolderLoadSet.omitted` | P9 |
| `refs.rs` · `relink.rs` leave folder entries out | ADR-0341 in 7 | `refs` never sees `FolderRef` (not an `AssetRef`); `relink.rs` is 9a (B2) and inherits it | P5 |
| Per-asset error attribution from a new additive port method | step 5 | §3-9 `merge_origins` · `validate::check_merged` (layers keyed by `AssetRef`) | P2 · P4 |
| Real-parser golden rows: YAML 1.1 scalars and non-string keys | step 5 | §8 `format` goldens through `YamlFrontMatter` + the codex port | P3 |
| Three claude preview divergences — fix here or keep recorded | step 5 | Kept recorded (B11) | — |
| (a) Produce the notices with the step-6 codes; same-name comparison with canonicalized paths | step 6 | `check_in` step 5, codes from `notice_codes`; canonical folder roots | P2 · P9 |
| (b) codex terminal plan: no instruction sources; merged file = priming only | step 6 | `check_in` step 7 · `prepare_in` | P9 |
| (c) Check-time `command_line_args` · `command_line_fits` · `skill_format` through the real parser with repair rows | step 6 | `check_in` step 5 · `validate::check_skill` · goldens | P3 · P4 · P9 |
| (d) Working-folder counterpart on the canonicalized folder | step 6 · ADR-0353 | `working_folder_blocks` §3-9 · `check_in` step 5 | P2 · P9 |
| (e) `prepare` leaves `last_applied_instructions_hash` `None`; `include_priming = true` ⇒ an `instructions_file` | step 6 | `prepare_in` | P9 |
| (f) `release` detaches every directory link at any depth | step 6 | `release` · `sweep_launch_dir` | P9 |
| Same-name notice policy (launch + notice) | ADR-0355 | `check_in` step 5 | P9 |
| §2-4 4 single-preset tag conflict | parent open | U5 (a) — §3-7 | P8 |
| §2-4 8 dim-line wording | parent open | Launch-reply half closed by D2 (no notice); schema notices U3 — §3-9 · §10 | P2 · P8 |
| ADR-0358 unknown keys survive an edit | ADR-0358 「영향」 | §3-4 write step 4 · §8 test | P3 |

**Deferred explicitly**: `relink.rs` · `edit.rs` · `config.rs` → 9a; `LibraryCommandHost` impl → 10a; `.git/info/exclude` · bundle ids → 9b; real fetch · version runner · refresh thread → 19; daemon wiring · boot sweep call · threading `LaunchScope` → 8; folder-line wording on screen, the dim display of rows not loaded for the backend, and the per-backend rows of the edit area → 13.

## 6. Concurrency and IO

The rule set is ADR-0324 / parent §1-4 flow ① 「잠금」 ①–⑤, implemented once in `gate.rs` + `lib.rs`:

- **One `std::sync::RwLock<State>`** (`State` = `Arc<Catalog>` · root state · schema generation seen). Reads take it once; a write never happens while holding the read lock (no upgrade). A scan or a re-check clones the `Arc<Catalog>` under a brief read lock, releases, builds the new catalog from that snapshot outside the lock, and swaps under the write lock.
- **Scan-generation `Mutex` + `Condvar`** (a leaf — nothing else is taken while holding it; the library lock is never held while waiting on it): ticket counter (FIFO), scan in progress, changes in progress, change generation, root generation, swap-in count, first-scan state ("raised" = one full scan swapped in), queued refresh generations with their range. `notify_all` on every release.
- **Other leaves** (recommended): the `Validator`'s `RwLock<Arc<SchemaSet>>` (held only to clone or swap the `Arc`), `BoundedIo`'s in-flight `Mutex`, each `Pending`'s answer `Mutex`, and a `LaunchIo`'s budget and answer `Mutex`es. None of them is held while taking another lock, while running an IO op, or while waiting.
- **Scan**: walk + parse **outside** the library lock; swap-in under the write lock **only if** the change generation is unchanged since the scan started (else rescan); the swap-in raises the swap-in count before releasing; it changes items inside its range only; inside the range "later read wins" by `read_seq`, and an item the listing did not contain counts as read 「absent」 at its folder's `listed_at` (§3-5 · must 6). Folders in `errors` keep their previous items.
- **First-scan join** at the entrance of every `Catalog`-reading method (`list` · `resolve_preset` · `resolve_agent` · later 9a edits) — before any ticket, count or lock. `check` is **not** an entrance and never waits for a scan (ADR-0324 decision 6 — tested with a scan held at the gate).
- **Refresh merging** (⑤): a `refresh` joins a covering generation that is queued and not yet walking; a covering walk already in progress ⇒ its own ticket. Before the first scan is raised, a folder `refresh` goes through the first-scan entrance (and re-checks "raised" on waking).
- **Conditional late merge** (④) — writers: `check`'s fresh bytes and `list`'s one-path safety net. Both note the three numbers **before** the disk read; in the short write lock: change or root generation moved ⇒ discard all; only the swap-in count moved ⇒ discard inserts of paths not in `Catalog`, keep updates of existing items when `read_seq` is later. **In every case the later `read_seq` wins over an item already in `Catalog`, even when the three numbers are unchanged** (recommended — an item a scan swapped in with a later read must not be replaced by an older read). **Admission** (must 5): a late merge and the safety net take only paths that pass `admits`, `first_link` (no link below the root) and the kind sniff — a path the scan would not admit is never merged; the safety net answers it as a skipped or broken row with the code `path-through-link` (a link) or not at all (a dot or temp name). `check`'s merge uses `try_write` and is skipped when contended (§3-8 step 6).
- **Schema re-check** = a change: plug the new set in → draw a ticket → count as a change → re-check settings items → raise the change generation inside the write lock. It does not trigger a first scan.
- **Folder IO — bounded, concurrent, never under the library lock** (D1 · U2): every stat · listing · canonicalize · file read on a folder entry, a native skill root, the working folder or the library root goes through `BoundedIo` with **one 5 s limit**. One launch (all its checks and its prepare) or one read computation (a `resolve_*`, a problem list, tree ⛔ over many agents) has **one 5 s budget**, charged only by time spent waiting on that IO (must 3). `check_in` has four IO batches (root; folder and native-root stats + working-folder canonical; listings; port reads + root canonicals) plus the overlay reads of library files; every batch waits only for the time left. One in-flight call per key; each distinct key once per launch; anything unfinished when the budget is spent = unreadable = ① for a folder entry (§3-8 for the root and native roots).
- **Library files** inside the root are read with the store directly (local by assumption — a network root is R4).
- **Pre-write checks** — `format::write::rewrite_lists` refuses out-of-range integers and re-parses its own output and compares it with the edited tree before returning (§3-4 steps 3 · 6); `FsStore::write_atomic` stats the parent first: missing ⇒ `NotFound` (nothing created). The gap between that stat and base's folder creation remains (B4).
- **Lock order** with the manager (ADR-0324): asset-library lock → manager locks, one direction; `check_in` holds no manager lock and calls only registry reads (`launch.rs:549-551`). `prepare_in` and `release` take no library lock.

## 7. Library survey per implementation part (parent §5-1 0a rule)

| Part | Choice | Version (B10) | Why (one line) |
|---|---|---|---|
| `LibPath` | relative-path | 2.0.x (2.0.3 current) | `/`-pinned relative paths with `strip_prefix`/`join`; our own validation on top (§1-12). |
| YAML read · front matter | serde-saphyr (`deserialize` · `strict_booleans` · `PassThrough` non-finite floats) | 2.0.0 chosen · 2.1.1 current | Line/column errors, duplicate keys refused, budget caps; `serde_yaml` archived (ADR-0336). |
| **Preset write** | **serde-saphyr `serialize`** (`to_string_with_options`) over our `Node` tree | same crate | ADR-0358: no new crate; `yaml-edit` dropped. `serde_json::Value` cannot be the tree here (premise 38). |
| Document tree | std (`Node` + a serde visitor) | — | serde-saphyr has no value type; `indexmap` + serde_json `preserve_order` would change serde_json for the whole workspace graph and still drop non-string keys. |
| Front-matter split | std (the library's split, ported from codex `extract_frontmatter`) | — | codex's exact rule; `gray_matter` differs (ADR-0336). |
| Schema check | jsonschema (`resolve-file`, `idna`) | 0.60.2 chosen · 0.61.1 current | No HTTP/TLS with defaults off; draft auto-detect; `instance_path()` for attribution. |
| Scan | walkdir | 2.5.0 (lock) | `follow_root_links` / `follow_links` / `filter_entry` give the U1 policy directly; a folder entry is yielded before it is read, which gives the per-folder sequence (must 6). |
| Excerpt | pulldown-cmark (default features off) | 0.13.6 | Event stream makes skipping code/tables/HTML/front matter simple. |
| Instructions hash · source digests | sha2 | 0.10 (lock 0.10.9) | Stable across restarts; byte-exact stale detection (must 7); 0 new packages. |
| Canonicalize | dunce | 1.0 (lock 1.0.5) | The manager's canonicalizer (`manager.rs:1566`) — `check` must judge the same value. |
| Atomic write · copy | base `file` | — | ADR-0335. |
| Path key · identity · junction · normalizer | platform `fs` | — | ADR-0325; normalizer added here. |
| Locks · gate | std `RwLock` · `Mutex` · `Condvar` | — | ADR-0324 (parking_lot rejected). |
| Bounded concurrent IO | std threads behind a `Spawn` seam + a `Condvar` per pending call, one per-launch budget charged by waiting | — | No crate cancels a blocking `metadata`; a worker per distinct key, joined by later callers, with a drop guard (B17). |
| Volatile cleanup | std + platform `unlink_dir` + agent `StdLaunchFs::is_link` | — | Detach-first is the stated contract (§1-5 6). |
| Schema fetch · version read | seams only | — | Real ones = step 19 (ureq 3.5.0 — ADR-0336). |

New packages vs draft 1: `yaml-edit` · `rowan` · `countme` · `text-size` are gone; `nohash-hasher` (serde-saphyr `serialize`) is new (premise 22); `dunce` is direct (no new package).

## 8. Test plan

Tests first per unit (TDD). Map to the parent §4 rows L779–L799. **Timing tests assert outcomes with gate-controlled fake ops** (a fake op that blocks until the test releases it, or never) and the budget's own accounting, not wall-clock ratios; the one wall-clock assertion per unit is a loose upper bound (limit + 1 s) on the real `std` path.

### 8-1. Per unit

| Unit | Seam / harness | What it pins |
|---|---|---|
| P1 base | table | `is_temp_name`: `<n>.tmp<pid>.<k>` · `<n>.corrupt.tmp<pid>.<k>` true; no number · leading zero · `+` · overflow false; `temp_owner` unchanged. |
| P1 platform | table, per-OS expected values through the helper | `normalize_folder`: `\\?\D:\x` = `D:\x` · `\\?\UNC\s\x` = `\\s\x` · `D:\x\..\Rules\` = `D:\Rules` · drive/UNC root keeps its separator · relative refused · `D:\Rules` and `d:/rules` key alike after normalize + `path_key`. |
| P1 path · backends | pure | `LibPath` refusal table · prefix swap · `AssetRef::parse` (`engram:/…` vs library; unknown built-in); `AgentBackends` answers exactly what `settings_check_for` / `settings_checks()` answer (both slots, case-insensitive). |
| P2 agent | pure (both slots) | `merge_origins` agrees with `merge` on every existing merge table; codex `None`; `working_folder_blocks` = the moved `cwd_reaches_codex_unchanged` table (step-6 `materialize` tests unchanged); `notice_codes` per backend × mode; `folder_listing_at_launch` per backend × mode; `carried` — claude empty for a folder with `CLAUDE.md` (while `loads` keeps the row), codex JSON = instruction + roots, codex terminal empty; `own_wording` rows for the four new codes; `LaunchScope` defaults delegate (Noop argv goldens unchanged); `LaunchScope::get` with a foreign type = `None`. |
| P3 format read | pure · golden | Item shapes round trip; tag in `presets`/`settings` = line problem; unknown tag word = line problem **and** an `unmodeled` row, a word added to a fake `Backends` is accepted; `folders` relative/`LibPath` = line problem; lenient `kind`; error line numbers; YAML 1.1 scalars under `strict_booleans`; `.nan` in an unknown key reads (premise 40); multi-document · mixed 0/2 indentation · duplicate keys · `!!tag` (§6-2 L989). |
| P3 codex skill golden | real `YamlFrontMatter` + `CODEX_SETTINGS_CHECK` | Upstream `parser_tests.rs` L56–85 rows · the research's pinned rows · repair rows (colon+space, `[` `{` `@` `` ` `` starts, unknown fields, trailing `#`, already quoted, block scalars) · `name: yes`/`on`/`off` · non-string keys · `description: 123` · BOM · whitespace-only block (M63 · §6-2 L1017). |
| P3 format write | pure · golden (each expected output pinned byte for byte, so removing the property fails the test) | An unknown top-level key keeps its value and position; **an unknown key inside an item map survives add-elsewhere · reorder (item moved first → last) · tag on · tag off · a `preset.save`-style list without origins** (the expected text holds the key in the same place); a map item losing its tag with an extra key stays a map, without one becomes a string; **unmodeled entries** (a number item, an unknown tag word) keep their node and their anchor after a reorder and after their anchor's removal (expected text pinned), and go only when listed in `drop_unmodeled`; `description` untouched; each of add · remove · reorder · tag on · tag off · empty list · first item into `[]` · absent list key on all five lists incl. `folders`; CRLF in ⇒ CRLF out · LF ⇒ LF · BOM kept · no BOM ⇒ none; strings `yes` · `on` · `null` · `1.0` · `012` · `~` · `-` · `:x` re-read as the same strings; a non-string key survives; an alias is written expanded with equal values; **`99999999999999999999` in an unknown key ⇒ `preset-write-number-range`, nothing returned** (the round-trip check alone passes this file — the test also asserts that, so it fails if step 3 is removed); `9223372036854775807` and `18446744073709551615` and `-9223372036854775808` survive exactly; **`.nan` in an unknown key ⇒ the edit succeeds** (fails if floats are compared with `==`); `-0.0` survives; a multi-document file is refused and left untouched; a forced serializer mismatch (test hook) ⇒ `preset-write-mismatch`; comments are gone (pinned). |
| P4 validate | fake `Backends` (with a fake third backend) · fake fetch · fake version read · fixture schemas | Syntax ⇒ ② with line; type/range ⇒ ②; `Ignore` silent at any depth; `Warn` ⇒ `settings-unknown-key`; older schema ⇒ `settings-schema-outdated` under `Warn` only (an `Ignore` backend with an older schema emits nothing); no schema ⇒ `settings-schema-unavailable`; merged-value error on the later asset's row, including the built-in settings layer as an `AssetRef`; real claude schema compiles and passes under 0.60.x with format validation stated; refresh keeps the held copy on failure and swaps the `Arc` on success (a reader holding the old `Arc` keeps it); a raw field named by the fake third backend is checked with no asset-library change. |
| P5 store/catalog/refs | `tempfile` (real) + `MemStore` | Real scan skips `.trash`, dot names, temp names; a leftover temp file is neither overwritten nor deleted; `write_atomic` with the parent gone = `NotFound`, folder not recreated; `trash` is a same-root rename; **U1**: a root that is a junction is followed, a junction / symlink folder and a file link inside are not entered or listed and appear in `skipped_links`; **a partial scan whose `under` is an internal junction (or lies below one) walks nothing and reports it** (must 5); `first_link` table (none · the leaf · a middle folder · the root itself not counted); **`listed_at`**: each folder carries the seq drawn before its listing (a `MemStore` hook records the order); a folder whose walk fails keeps its previous items; Unsupported identity ⇒ `None`; re-read only on tag change or racy mark; preset-side reverse index; `reference_count` = presets + agents (`same_path`); closure excludes `engram:/` and folders; excerpt drops front matter · code · tables · HTML. Scan cost on a generated 5,000-item tree (`#[ignore]` measure — §6-2 L1004). |
| P6 folder IO | fake `FolderOps` (gate-controlled) · fake `Spawn` · shortened limit | Stat / list / canonical / read answer through the worker; **a listing answers**; N held keys in one batch end when the budget does, and the budget's accounting shows one limit spent, not N; **two batches share one budget** — batch 1 held until 0.9 × limit (gate released by the test), batch 2's held listing is cut at the shared deadline ⇒ `Unreadable("timeout")`; a batch started with nothing left returns at once; **time outside IO is not charged** — the test sleeps past the limit between two batches, the second still gets the time left (must 3); **a held file read**: N sequential `read_one` calls with the first held ⇒ the sequence ends inside the budget, later reads answer `TimedOut` at once; an answer in the `LaunchIo` is reused (op call count = 1); `read_one(fresh)` asks again; **spawn failure** ⇒ `Unreadable("spawn failed…")`, entry removed; **worker panic** ⇒ `Unreadable("worker panicked")`, waiters woken, entry removed; **recovery** — an abandoned call completes later, its entry is removed, the next caller starts a fresh op and gets `Ok`; a joiner within the limit gets the pending answer; a joiner after one full limit gets `Unreadable("timeout")` at once without a second op; **the in-flight map is empty** after every test; **a folder created after a `Missing` answer is seen by the next `begin()`**; listing cap ⇒ `truncated`; links not entered (real `std` op on `tempfile`). |
| P7 service/gate | `MemStore` with a scan counter · a gate that holds a scan · failure injection | `list` never scans after the first scan; first-scan join from every entrance; refresh merging only when the range covers; refresh before the first scan goes through the first-scan entrance; FIFO both ways; a scan that saw a change generation move rescans; late merge discard rules; **a late merge with all three numbers unchanged keeps the item with the later `read_seq`** (both orders); **listing-sequence interleavings, deterministic through the gate and `MemStore` hooks**: (i) a check reads X (seq 10), X is deleted, the scan lists X's folder (seq 12) ⇒ X is gone after both land, in either landing order; (ii) the scan lists the folder (seq 12, X absent), X is created and a check reads it (seq 15) ⇒ X is present in either landing order; the safety net and a late merge refuse a path through an internal junction (`path-through-link`), a dot name and a temp name; schema re-check as a change; a scan builds from an `Arc` snapshot while a reader holds the read lock (no deadlock, no guard held through the build); `list` rows carry reference counts; `refresh` returns skipped links and walk errors. |
| P8 resolve | `MemStore` lookup · fake ports · fake `Backends` · fake folder status | Parent §4 L779 rows (D1 order · dedup · cycle · conflicts on loaded skills only · G51 per-preset tag order · only the agent backend's raw field · notices · same reasons whoever launches · block ④); **an item tagged for another backend and a settings asset without this backend's field produce no notice** (D2); **U5**: common×common, common×tag, same-tag×same-tag conflict, claude-tag×codex-tag does not, and a fake third backend's tag behaves the same; folder entries (spelling dedup · ① for missing/unreadable on every backend and mode · ② where `folder_entry_blocks` · no notice for a folder with nothing to load); a schema error in another backend's field does not block; `RootState::Unavailable` ⇒ ④ with a library preset; built-in missing ⇒ no priming, no block. |
| P9 launch | `FsStore` + `tempfile` · real ports · fake `Backends` · gate-controlled fake `FolderOps` | `check` blocks (each reason) and passes; a priming-only mode plan has no instructions; a `loads.skills == false` mode plan has no skills and its omitted notice names them; a priming that does not fit ⇒ `Broken`; working-folder rule on the canonicalized folder (a junction cwd), and a canonical timeout falls back to `profile.cwd`; same-name notice with a canonical duplicate removed (only roots canonicalized — op count); notice codes taken from `notice_codes` (a fake backend with its own codes); **a claude plan with a folder holding `CLAUDE.md` has no `folder:` source and makes no listing call** (must 9); one unreachable folder blocks the launch with ①; **a held file read inside a folder entry ⇒ the folder is ① and `check` ends inside the budget** (must 1); **a write lock held across the overlay step for longer than the limit, then released ⇒ a healthy folder is `Ok`** (must 3); **folder IO runs with the library lock released** (the fake op asserts `try_write` succeeds); **`check` completes while a scan is held at the gate**; **the scope is shared** — `check_in` twice + `prepare_in` on one scope ask each folder once (op counts) and spend one budget; a contended write lock ⇒ the merge is skipped and `check` still answers; an overlay path through an internal junction ⇒ `path-through-link` ②, never read; a truncated listing ⇒ `folder-listing-partial`; `prepare` `Stale` on a vanished source, on a vanished folder instruction file, and **on changed bytes with unchanged size and mtime** (the test restores the mtime — must 7); `prepare` takes no library lock (holds the write lock in the test and still finishes); `include_priming` ⇒ file; `merge_instructions` gives the same text to `command_line_fits` and to the written file; hash stable; `release` detaches junctions at depth 1 and 3 and the library skill folder survives; idempotent; `sweep_launch_dir` the same; one end-to-end real kit → `backend::materialize` for both registered backends on a temp folder. |

### 8-2. The four CI gates and the `/qa` binding (P1)

1. Test line — a `ci.yml` backend-job step `cargo test --locked -p engram-dashboard-asset-library` (no `--test-threads=4`: no test spawns a child process).
2. Dependency cap — a gates-job step on the `cli gate 1` template (`ci.yml:1207`): `cargo tree --locked -p engram-dashboard-asset-library --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | awk '{print $1}' | sort -u` equals exactly {agent · asset-library · base · command · platform}.
3. Messaging isolation — `ci.yml:561` alphabet gains `asset_library`.
4. `use tauri` 0 — a gates-job step on the agent template (`ci.yml:539`), with `test -d crates/engram-dashboard-asset-library/src` first.

`.claude/skill-bindings/qa.md` 「모듈별 명령」: add `asset-library` to the row `base · command · protocol · messaging` (`cargo test -p engram-dashboard-<이름>`, no thread flag). No new 「돌릴 것 표」 row, no lane. The license gate is run locally in P1 and P3 (premise 25).

### 8-3. Review check — backend extensibility (every unit's code review)

- `rg -n -i '\b(claude|codex)\b' crates/engram-dashboard-asset-library/src` — every hit must be inside a `#[cfg(test)]` module, a doc comment that describes today's port answers, or built-in content; **no hit in a branch, a match arm, a string the code emits, or a field name**.
- `rg -n 'settings_check_for|settings_checks\(' crates/engram-dashboard-asset-library/src` — hits only in `backends.rs` (`AgentBackends`).
- No `enum` over backends; every backend-dependent branch reads a port answer (§1-1 list).
- Each unit that branches on backend behaviour has a test with a **fake third backend** injected through `Backends` (P3 · P4 · P8 · P9 rows above).
- Main may later turn the `rg` lines into a CI gate once tests live in separate files (B21).

## 9. Implementation units

Sequential unless marked. Each ends green: build, its tests, full regression, the four gates. Sizes are rough (prod / tests, lines). Draft 2's order did not build (must 8): `Catalog::build` needs the `Validator`, `resolve`'s entrances need `FolderStatus` / `BoundedIo`, and `list` needs `refs` (premise 46). Draft 3 moves `validate` before the catalog, `refs` into the catalog unit, and the folder IO before the service.

| Unit | Files | Size | Needs | Ends green because |
|---|---|---|---|---|
| **P1** skeleton + leaves + gates | base `file.rs` · platform `fs.rs` + `fs/normalize.rs` · new crate `Cargo.toml` · `lib.rs` (header) · `backends.rs` · `path.rs` · root `Cargo.toml` (+ "11 members" comment) · `Cargo.lock` · `ci.yml` (4 gates) · `qa.md` | 500 / 500 | — | Nothing calls the crate; base/platform additions are new functions. |
| **P2** agent port additions | `backend/settings_check.rs` · `backend/claude/settings.rs` · `backend/codex/settings.rs` · `backend/codex/launch.rs` (rule → shared helper) · `launch.rs` (`stamp`, `LaunchScope` + three default methods, `own_wording` rows) · the 6 `LaunchPlan` and 9 `FolderLoadSet` literals | 400 / 450 | — | Additive methods with defaults or for both slots; `materialize` keeps the same rule; goldens unchanged. **May run beside P3 and P6.** |
| **P3** format: read · front matter · write · codex golden | `format/mod.rs` · `read.rs` · `frontmatter.rs` · `tree.rs` · `write.rs` | 1,050 / 1,600 | P1 | Pure functions + goldens. |
| **P4** validate | `validate.rs` · fixtures (claude schema; codex schema if premise 36 resolves) | 650 / 850 | P2 · P3 | Pure checks over fake ports and fixtures. |
| **P5** store · scan · catalog · excerpt · refs | `store.rs` · `catalog.rs` · `excerpt.rs` · `refs.rs` | 1,050 / 1,250 | P3 · P4 | No service yet; `Catalog::build` takes P4's `Validator`. |
| **P6** folder IO | `folder.rs` | 450 / 700 | P1 (agent `FolderEntry` exists) | Fake ops and spawner; nothing calls it. **May run beside P3–P5.** |
| **P7** service + gate | `gate.rs` · `lib.rs` (`LibraryService`, `list`, `refresh`, schema re-check, late merge) | 950 / 1,200 | P5 · P6 (the constructor takes `BoundedIo`) | Concurrency tests with held scans; nothing outside calls it. |
| **P8** builtin + resolve + entrances | `builtin.rs` · `resolve.rs` · `lib.rs` (`resolve_preset`, `resolve_agent`) | 900 / 1,250 | P6 · P7 | Pure resolve over `Lookup`; entrances reuse P7's join and P6's budget. |
| **P9** launch | `launch.rs` · `lib.rs` (`LaunchKitSource` impl) | 1,000 / 1,350 | P2 · P8 | `impl LaunchKitSource` tested end-to-end on temp folders; no daemon caller. |

**Interfaces between units** (frozen when the earlier one lands):
- P1 → `Backends` / `AgentBackends`, `LibPath` / `AssetRef` / `BuiltinId` / `PresetItem` / `ItemOrigin` / `FolderRef`, `is_temp_name`, `normalize_folder`.
- P2 → `merge_origins`, `working_folder_blocks`, `notice_codes` / `NoticeCodes`, `folder_listing_at_launch`, `FolderLoadSet.carried`, `LaunchPlan.stamp`, `LaunchScope` + `begin_launch` / `check_in` / `prepare_in`, the four new codes' wording.
- P3 → `parse_preset(text, backends)` / `parse_settings` / `sniff_kind` / `PresetDoc` (incl. `unmodeled`) / `YamlFrontMatter` / `split_front_matter` / `library_skill_name` / `conflict_key` / `format::options()` / `Node` / `rewrite_lists` / `PresetLists` (the writer used by 9a only).
- P4 → `Validator` (`current()`, `refresh_schemas`) / `SchemaSet` / `SettingsVerdict` / `check_settings` / `check_merged(AssetRef layers)` / `check_skill` / `SchemaFetch` / `CliVersionRead`.
- P5 → `LibraryStore` (incl. `first_link`) / `admits` / `ScanListing` (incl. `listed_at`) / `ContentTag` / `TrashBundle` / `Catalog` / `Item` / `ItemContent` / `refs::{closure, referrers_of, reference_count}`.
- P6 → `BoundedIo` / `FolderOps` / `Spawn` / `IoKey` / `IoBudget` / `LaunchIo` / `FolderStatus` / `FolderListing`.
- P7 → `LibraryService::new(values, store, io)`, `LibraryValues`, the gate API (`first_scan_join`, `with_read`, `late_merge(numbers, f)`), `list` / `ListAnswer` / `ListRow`, `refresh` / `RefreshReport`.
- P8 → `Builtins`, `Lookup` / `Looked` / `ItemView` / `Target` / `RootState` / `Resolution` (two phases) / `ResolvedSettings` / `ResolvedFolder` / `Problem`, `resolve_preset` / `resolve_agent`.
- P9 → the `LaunchKitSource` impl, `merge_instructions`, `sweep_launch_dir`.

Total ≈ 6,950 prod / 9,150 tests (draft 2: 6,300 / 8,050). One commit per unit (B18). After the units: `/engram-review code` per unit (P6, P7 and P9 deep), `/engram-qa`, doc pass (§12). *Stop (after)* per the parent.

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
| **Backend extensibility** — tags, raw-field names and selection are strings matched through the registry, reached only through `Backends`; no backend literal in asset-library outside tests and built-in content | User 2026-11-11 (§1-1) |
| **U1 = (a)** — follow the root if it is a link; do not enter links inside the library; `library.refresh` names the skipped links. **Applies to every path** into `Catalog` (partial scan, safety net, overlay reads, late merges — review round 1 must 5) | User 2026-11-11 「애초에 바깥폴더를 못넣게 했잖아. 근데 어찌어찌 들어가더라도 건너띄는게 좋지 않음?」. Convention (main 2026-11-11): generic walkers (walkdir default, ripgrep) do not follow links; skill loaders (codex `codex-rs/ext/skills/src/loader/discovery.rs`, the Claude Code CLI) do — (a) differs from the loaders on purpose: G37 and 9b's move/delete would otherwise reach the far side of a link. Rejected: (b) follow every link · (c) list links as an error row. |
| **U2 = folder IO concurrent; over the limit = unreadable = block ① for the whole launch; listing cap 10,000 (truncate + mark partial, no block)** — the limit value is now D1's | User 2026-11-11 (「너무 디테일하게 하지말고 심플하게 가는게 좋아보이는데?」; block rule 「그냥 하나 막혀도 전체 막는게 좋겠지? 구지 무리하게 실행하는것보다는」). Concurrency across paths = main 2026-11-11. Convention: cap — VS Code search `limitHit`, codex `MAX_SKILLS_ENTRIES_PER_ROOT` + `inventory_complete`; timeout — none found (Claude Code issue #48361 hangs on an offline drive). |
| **D1 — one 5 s folder-IO budget per launch** (replaces U2's 10 s): `check_before_create` → `prelaunch` (every attempt) → `prepare` share one budget and one answer set through `LaunchScope`; a read computation outside a launch gets one 5 s budget; the budget is charged only by IO waiting; a blocked launch answers inside the 7 s bus slot (premise 41) | User 2026-11-11 「7초 동일하게 하던지. 이건 관행 맞춰서 너가 대충해도 될듯.」 — main decided the detail (5 s leaves ≥ 2 s of the 7 s `DEFAULT_DEADLINE` for parsing, locks and the reply). Rejected: 10 s per computation (draft 2 — up to 3 checks + prepare could pass 7 s several times over). |
| **D2 — no notice for a settings asset without this backend's field (`settings-backend-field-absent`) or for an item tagged for another backend (`item-other-backend`)**; both codes, their wording and their tests are removed. The remaining new notices: `settings-schema-unavailable` · `settings-schema-outdated` · `settings-unknown-key` (+ `folder-listing-partial`, B22) | User 2026-11-11 「음? codedx칸이 없을수도 있지 왜 알림을해?」 and 「지금 노출 안된상태에서 Add 하면 보여주는 걸로 해쌎ㄶ아. 왜냐하면 모델이 너무 많아질수 있으니.」 — per-backend fields and rows exist only when the user adds them (the step-13 hand-over, §12), so their absence is the normal state, and a tag exists to be skipped; with more backends these notices would multiply. Reverses parent §1-3 6 ② for the launch-reply notice list (§12). |
| **U5 = (a)** — in a single preset, a same-name pair conflicts only if some backend would load both; two different tags never conflict | User 2026-11-11 「A」. Closes parent §2-4 4. |
| **U6 = (c)** — drop `yaml-edit`, rewrite with serde-saphyr | ADR-0358 (options (a) list as synthesized and (b) hold the dependency rejected by the user) |
| App wording does not use 「싣다 / 실리다」 | User 2026-11-09 (`trd-codex-translation.md` L725 · L571 · `prd.md` L113 · ADR-0357) — every new Korean sentence in this TRD follows it (§10-2 U3 · U4 · B22). |

### 10-2. Main decided or can decide (recommendation + reason)

| # | Question | Decision / recommendation | Reason |
|---|---|---|---|
| **U3** | New notices and their carriage | **Decided by main 2026-11-11 (user: wording detail) = (a)**, narrowed by D2: the launch reply carries the schema notices (once per agent — R58) and the views show them, each with its own fact-only Korean sentence in `own_wording`: `settings-schema-unavailable` (「알림: 설정 <이름>의 스키마를 확인하지 못했습니다(스키마 없음).」) · `settings-schema-outdated` (「알림: 설정 <이름>의 일부 키는 이 판 스키마로 확인하지 못했습니다.」). Rejected: (b) views only · (c) generic wording | The parent's design (§1-3 6 — ③ is carried by the launch); once-per-agent stops repetition; the generic 「의도된 동작입니다」 is wrong for a missing schema. Names come from `args`, not code (§1-1). |
| **U4** | codex-style unknown setting key | **Decided by main 2026-11-11 = (a)**: notice `settings-unknown-key` (「알림: 설정 <이름>에 스키마에 없는 키가 있습니다: <키>」), emitted whenever a backend's `unknown_keys()` is `Warn` — not keyed to codex. Rejected: (b) not shown · (c) a warning category (ADR-0331) | ADR-0332 keeps it ⚠, ADR-0331 has no warning category; a notice is the only non-blocking channel. Amends ADR-0331 「영향」. |
| B1 | File split and names | `format/` folder (read · frontmatter · tree · write); `backends.rs`; `gate.rs`; `excerpt.rs`; keep `command` in the manifest now | One reason per file; gate ② final from day one. |
| B2 | What of §1-4's file list is step 7 | `relink.rs` · `edit.rs` · `config.rs` · the profile-edit late cell → 9a; `ops.rs` → 9b | They write files or profiles; step 7 is read + launch + the writer primitive. |
| B3 | Storage seam completeness | Declare and implement all methods now (`rename` · `copy` · `trash` thin, tested; `TrashBundle` a placeholder newtype until 9b) | Seam frozen once. |
| B4 | Missing-parent write | Stat the parent, `NotFound` if missing; record the narrow gap | M65 "implementation judgment". |
| B5 | Lenient `kind` and item shapes | Line scan for a top-level `kind:` before the typed parse; items via a value enum, not `untagged` | `Spanned` is lost inside `untagged`/`flatten` (§6-2 L1018). |
| B6 | Working-folder rule's port shape | `working_folder_blocks(cwd, mode)`; the codex rule in one helper for both callers | One rule (ADR-0353); mirrors `folder_entry_blocks`; also closes the duplication T-92 names for this guard. |
| B7 | How `prepare` detects "changed" | `LaunchPlan.stamp: Option<String>` (opaque) holding the **SHA-256 of every file whose bytes `check` used**; `prepare` re-reads and compares — no lock — 6 literals | Byte-exact (must 7: mtime + size + identity miss a same-length in-place edit on a coarse-timestamp file system); stateless and explicit; sha2 already chosen (ADR-0336). |
| B8 | Code names (asset-library's own) | Blocks: `not-found` · `folder-not-found` · `yaml-syntax` · `kind-unknown` · `wrong-kind` · `preset-cycle` · `builtin-in-library-preset` · `backend-tag-unknown` · `folder-path-relative` · `folder-path-cli-unsafe` · `settings-schema` · `skill-name-invalid` · `skill-name-conflict` · `library-unavailable` · `path-through-link`; writer `preset-write-mismatch` · `preset-multi-document` · `preset-write-number-range`; notices = U3 · U4 · B22. Port codes pass through unchanged and are owned by the backend (e.g. `codex-working-folder-unsafe`, `codex-skill-*`, `command-line-too-long`) | Kebab-case; one name per cause; no backend name in asset-library's own codes (§1-1). |
| B9 | Instructions hash (T6) | SHA-256 hex of the merged file bytes (sha2 0.10) | Stable across restarts; in the lock. |
| B10 | Versions | `jsonschema 0.60.x` (ADR-0336); serde-saphyr: the current 2.x **only after** the P3 goldens (read and write, incl. premises 39 · 40) pass on it, else pin `=2.0.0`; `relative-path 2.0`; `sha2 0.10`; `dunce 1.0` | A reader/writer bump is cheap to verify with the goldens. Report the resolved versions (CLAUDE.md 「의존성」). |
| B11 | Step-5 claude preview divergences | Keep recorded (tracking) | Agent code, preview-only. |
| B12 | T-96 (no claude `--add-dir` count) | Not in step 7; stays tracked | Needs a measurement first. |
| B13 | A backend with no settings check (Shell) in `check` | Noop plan (`real: false`) | It takes no preset (§1-5); argv byte-identical. |
| B14 | `builtin\` names and merged file name | `presets/default.yaml` · `instructions/priming.md` · `settings/default.yaml`; merged = `instructions.md` | Library shape (G1); step 17 creates them. |
| B15 | Built-in missing | Log a warning; no notice | M7; the release zip always ships it. |
| B16 | Excerpt rule | First paragraph text, whitespace collapsed, 200 characters | The client tunes display length. |
| B17 | Bounded IO mechanism | Worker per distinct key behind a `Spawn` seam, one in-flight call per key, joiners within one limit, a completion guard on every exit, answers kept only in the asking `LaunchIo`; **one 5 s budget per launch, charged by IO waiting** (D1 · must 2 · must 3) | The parent's mechanics + main's concurrency + D1; the user was told "however many folders, a blocked launch answers inside the bus slot". |
| B18 | Commit granularity | One commit per unit, `S25: feat(asset-library): …` | Each unit reviewable and green alone. |
| B19 | Version runner and fetch | Seams only; real ones in 19 | ADR-0311 decision 4. |
| B20 | Notice codes without backend knowledge | `SettingsCheck::notice_codes(mode)` (§3-9); asset-library emits a code only when the port gives one | Draft 1 would have picked the three codex-named codes by knowing the backend (§1-1). |
| B21 | Make §8-3's `rg` lines a CI gate | Later, once tests can be excluded mechanically; review check now | Inline `#[cfg(test)]` modules make a file-level `rg` gate noisy. |
| B22 | A listing past the 10,000 cap at launch | Notice `folder-listing-partial` (`args` `{folder}`) — 「알림: 폴더 <폴더>의 항목이 10,000개를 넘어 일부만 확인했습니다.」; no block (U2) | The port cannot name the entries it needs before it sees a listing, so "read the fixed names first" would need a new port answer for one rare case; a notice keeps the truncation visible. Wording = main's call (as U3). |
| B23 | Built-in ids | **Closed set** (`DefaultPreset` · `Priming` · `DefaultSettings`) — a narrowing of ADR-0329 decision 5 (「기본」 *can* load per-backend priming through the tag): a second priming file needs a new `BuiltinId` and a file under `builtin\` (an asset-library content change, not a backend change — §1-1 does not apply). Record in ADR-0329 「영향」 in the doc pass | The parent fixes three short ids (`engram:/default` · `engram:/priming` · `engram:/settings/default`) that do not map to `builtin\` paths by one rule; opening the namespace now designs a feature nobody asked for. |
| B24 | A native skill root that does not answer within the budget | Left out of the same-name comparison, logged; no block | ① is "a folder the preset names cannot be read" (ADR-0341 decision 6); a native root is not named by any preset and only feeds an advisory notice. |

### 10-3. User decisions needed

None open. (History: draft 1 asked U1–U6; U1 · U2 · U5 · U6 were answered by the user on 2026-11-11, U3 · U4 were left to main. After review round 1 the user decided D1 (detail by main) and D2 — §10-1.)

### 10-4. Open items touching step 7

| Item | Must step 7 resolve it? |
|---|---|
| Parent §2-4 4 | Resolved — U5 (a). |
| Parent §2-4 8 (notice wording) | Dim-line half: no launch notice (D2); the dim display in views is 13's. Schema notices: resolved — U3. Folder-line wording: no (13). |
| ADR-0331 「영향」 codex unknown-key mismatch | Resolved — U4 (a); the ADR text is amended in the doc pass (§12). |
| Parent §2-4 1 · 2 · 6 · 7 | No — 17 · 8 · 11 · 19. |
| Parent §6-2 rows tagged "step 7" (L989 unrun YAML shapes — now for the reader and the round trip · L1003 excerpt · L1004 scan cost · L1009 `strict_booleans` · L1015 FAT identity · L1016 walkdir/junctions — U1 · L1017 codex check parity · L1018 schema/Spanned · L1019 stat timeout — D1 · U2) | Yes — measured by §8 tests; results go back to §6-2. |
| Parent §6-5 6 · 10 · 11 (comment gaps, gate refusal, flow-list comments) | Moot under ADR-0358 — strike in the doc pass. |
| Tracking T-96 · T-101 · T-92 · T-62 | T-96/T-101 no (B12); T-92 partly closed by B6; T-62: the new packages' notices regenerate (no synthesized entry expected — premise 25). |

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | The gate is the hardest code in S25 and its bugs are races | P7 alone, deep review, held-scan and injected-order tests (parent §4 L783), the two listing-sequence interleavings; rules already written (ADR-0324). |
| R2 | serde-saphyr vs codex's serde_yaml 0.9.34 edge differences | The P3 codex golden; record divergences that cannot be closed (§6-2 L1017). |
| R3 | Per-file `file_identity` makes a large first scan slow; a first-scan join inside `library.list` meets the 7 s bus deadline (premise 41 — draft 2 said 30 s) | P5 measures 5,000 items; the first scan does not block launch (`check` is not an entrance); report before 10a if slow. |
| R4 | A library root on a network share blocks catalog reads on IO | Out of support this edition (default root is local); `BoundedIo` covers the root stat, folder entries, native roots and the working folder only. Note in parent §6-3. |
| R5 | A whole-file rewrite surprises a user who hand-edits presets (comments vanish, layout changes) | Decided (ADR-0358); `engram help library` and the priming should say an edit through Engram rewrites the file (10a / 17 text — §12). |
| R6 | The serializer writes a value that re-reads differently (data loss) | The round-trip check refuses the write (§3-4 step 6); the P3 golden covers ambiguous scalars, numbers, non-string keys, unknown keys, NaN. |
| R7 | Abandoned IO workers pile up on many distinct unreachable paths | Bounded by distinct keys; one in-flight per key; a call older than one limit is not joined but also not restarted until it returns; logged. |
| R8 | A folder whose stat answers late (a share that wakes after 4 s) leaves little of the 5 s budget for its listing and reads; they are cut and that folder counts as unreadable ⇒ ①, although it exists. A `prepare` that runs after a slow check has only the rest of the budget for its fresh folder-file reads ⇒ `Stale` ⇒ `LaunchRaced` | Accepted as the cost of a fixed cap inside the bus slot (D1 · U2 — block rather than run partly). The log says "timeout"; a retry usually passes (the share is awake and the next launch has a fresh budget). Only folders whose listing the backend needs at launch are affected (`folder_listing_at_launch` — claude has no second phase). |
| R9 | The real claude schema does not compile or pass on jsonschema 0.60.x | P4 finds out first; fallback = syntax-only + `settings-schema-unavailable`; report. |
| R10 | A backend added later needs a port answer asset-library does not yet ask for | §1-1 lists every port answer used; §8-3 fake-third-backend tests show the gap early. |
| R11 | A user expects linked content to work (U1) | `refresh` names skipped links; references through a link get `path-through-link` instead of a bare 「찾을 수 없음」; decided by the user. |
| R12 | Step 7 is large (≈ 16k lines incl. tests) | Nine green units; P2 and P6 can run beside P3–P5. |
| R13 | A preset holding an integer beyond 64 bits (or an integral float ≥ 2^63) can no longer be edited through Engram | `preset-write-number-range` names the key; the user edits by hand. Such a value in a preset is unlikely (no Engram key takes one). |
| R14 | Until step 8 threads `LaunchScope`, each `check` / `prepare` call makes its own budget, so a create launch could spend up to 4 × 5 s | No caller exists before step 8 (§1); step 8's hand-over makes the scope mandatory and its source-guard tests pin it (§12). |
| R15 | The budget does not count lock waits (must 3), so a library write lock held long (ADR-0324 M47 — trash emptying during a delete) can still push a launch past 7 s | Already accepted by ADR-0324 「영향」 (M47) with its own next step (move bundles before sending); D1 bounds only what folder IO adds. |

## 12. Hand-over to the parent TRD / ADR changes

Main copies these after landing (the parent TRD lines that change because of ADR-0358 and the 2026-11-11 decisions are listed first):

- **§1-2 L81** (the preset-file heading) — rewriting = a whole-file rewrite with serde-saphyr's serializer; comments, blank lines, quoting not kept; unmodeled keys (top level and inside items) and unmodeled list entries kept; an integer beyond 64 bits refuses the edit (ADR-0358). **L84** (the example's first comment) — replace with a comment saying an Engram edit rewrites the file and drops comments.
- **§1-4 `format.rs` row (L192)** — replace the `yaml-edit =0.3.5` text, the three conditions, the reorder detour and the pre-write gate by: read = serde-saphyr; write = parse into the order-keeping `Node` tree, number guard, rebuild the five lists by item identity (origin, else first unused same spelling) keeping each item's node, pass unmodeled entries through, serialize, re-parse round-trip check (`preset-write-mismatch`), keep line ending and BOM, refuse multi-document files; the front-matter parts unchanged.
- **§1-12** — the `yaml-edit` row → "Rejected (ADR-0358)"; the serde-saphyr row: features `["deserialize","serialize"]`, uses = reading **and** rewriting presets, new indirect package `nohash-hasher`; `dunce` direct; version notes (B10).
- **§4 L784** (`format` test row) — the comment-preservation goldens become the §8-1 P3 write rows.
- **§5-1 0b (L828)** — append: result superseded 2026-11-11 by ADR-0358.
- **§5-2 step 7 (L893)** — "preset edit writing = `yaml-edit =0.3.5` + three conditions …" → "preset writing = whole-file rewrite with serde-saphyr (ADR-0358)"; add the backend-extensibility rule, `Backends`, and the port additions; then a "Step 7 landed" block.
- **§6-2 L989 · §6-5 6 · 10 · 11 · §6-3 (the gate-blind shape rows)** — moot under ADR-0358; the unrun shapes are re-pointed at the reader and the round trip.
- **§1-3 「Folder entries in resolution」 · §1-4 `folder.rs` row** — one 5 s limit for stat · listing · canonicalize · file read; IO concurrent across paths; **one 5 s budget per launch** (all checks and the prepare of it — D1) or per read computation, charged only by IO waiting; anything unfinished = unreadable = ① (worst case: a blocked launch answers inside the 7 s bus slot); listing cap 10,000 with `truncated` and the `folder-listing-partial` notice (U2 · B22). Replaces every "10 s".
- **§1-3 6 ②** (dim rows in the notice list) — **reversed for the launch-reply notice list** by D2 (user 2026-11-11): no notice for a settings asset without the agent's backend field or an item tagged for another backend. Whether the views show such rows dimmed is a step-13 screen matter.
- **§1-2 kind table / §1-4 `store.rs` row** — links inside the library are not entered, listed, read or merged on any path (scan, partial scan, safety net, launch overlay, late merge); only the configured root may be a link; `library.refresh` names the skipped links (U1).
- **§1-4 flow ① 「잠금」 ④** — the per-folder listing sequence (`listed_at`) and "later `read_seq` wins in every case"; `check`'s merge is `try_write`.
- **§1-3 5 · §2-4 4** — closed by U5 (a). **§2-4 8** — D2 + the U3 codes and wording. **ADR-0331 「영향」** — the codex unknown-key mismatch closed by U4 (a). **ADR-0329 「영향」** — the closed built-in id set (B23).
- **§1-5 settings-check port** — four additive methods `merge_origins` · `working_folder_blocks` · `notice_codes` · `folder_listing_at_launch` and the field `FolderLoadSet.carried` (B6 · B20 · must 9: the launch carries `carried`, never `loads`; claude's folder set stays preview-only); `LaunchPlan.stamp` = source digests (B7); the design rule of §1-1 (backend extensibility, `Backends`) as a paragraph under 「Backend settings-check port」.
- **§1-5 `LaunchKitSource`** — `LaunchScope` and the default methods `begin_launch` · `check_in` · `prepare_in` (D1).
- **Step 8 (D1 — mandatory)**: the manager calls `kits.begin_launch()` once per launch — at the start of the create spawn, before `ensure_registered` → `check_before_create` (`manager.rs:1345`); at `prelaunch` for restore and resume — and passes that one scope to `check_before_create`, to every attempt of `prelaunch` (`prelaunch.rs:50-77`) and to `prepare` (`manager.rs:1549`) via `check_in` / `prepare_in`. The scope is dropped when the launch ends (success or failure). The source-guard tests in `manager/prelaunch.rs:1845-1938` (which count `.kits.check(` / `.kits.prepare(`) move to the `_in` names. Also: `sweep_launch_dir` before plugging the port; `LibraryValues` incl. `backends = AgentBackends` and `io_limit` = 5 s.
- **Step 13 (client) — UI requirement from the user (2026-11-11)**: in the preset edit area, per-backend rows/fields (a backend tag on an item, a backend's raw field in a settings asset) appear only when the user adds them — not one column per backend always shown. User: 「ui 기준으로는 모든 칸에 다 나오면 보기 불편하니깐 추가해야지 나오는 형태? … 이건 ui 할때 자세히 해도 될듯」 and (D2) 「지금 노출 안된상태에서 Add 하면 보여주는 걸로 해쌎ㄶ아. 왜냐하면 모델이 너무 많아질수 있으니.」. The list of choices comes from the backend registry.
- **Steps 17 · 19 · 9a · 10a** — step 17 uses the B14 names; step 19 implements `SchemaFetch` · `CliVersionRead` and calls `refresh_schemas`; 9a builds `preset.*` on `rewrite_lists` (passing `origin` through every operation, `drop_unmodeled` for an explicit removal of a broken line, and `preset.save` lists without origins); 10a/17 text — `engram help library` and the priming say an Engram edit rewrites the preset file (R5).
- **ADR candidates (`/engram-adr`)** — U1 (links inside the library are skipped on every path); U2 + D1 (concurrent folder IO, one 5 s budget per launch charged by IO waiting — amends ADR-0341's "value = implementation judgment"); U3 + U4 + D2 (the new notice codes, and the two not added — amends ADR-0331 「영향」, extends ADR-0357's facts-only rule); backend extensibility for asset-library (§1-1) with `Backends` and B20 as its port consequences (ADR-0334 impact note).
- **CLAUDE.md (step 18 input)** — 「백엔드 모듈 맵」: the `asset-library` entry (what it holds · its four dependencies · no Tauri/network/test feature · no backend literal · registry only through `Backends`); 「빌드·검증 명령」: `cargo test -p engram-dashboard-asset-library` and the gate names; 「의존성」: the new packages (serde-saphyr with `deserialize`+`serialize` · jsonschema · relative-path · pulldown-cmark · walkdir direct · sha2 direct · dunce direct; indirect `nohash-hasher` · `granit-parser` · `annotate-snippets` · `encoding_rs_io`) — report to the user.

## 13. Review record

| Date · depth | Reviewers | Verdict | What changed |
|---|---|---|---|
| 2026-11-11 · deep | lead + concurrency axis + codex gpt-6.1-sol xhigh | **Round 1 FIX — 11 must applied in draft 3** | See below. |

Round 1, must findings (all applied, each fact checked against code first):

1. Bounded file reads — §3-8 `read_one` on the launch budget; a read timeout ⇒ the folder entry is ① (the port's `unsure` row does not decide) — premise 43.
2. `BoundedIo` entry lifetime — answers live only in the asking `LaunchIo`; worker removes its entry; completion guard on panic and spawn failure; no join after one full limit; `FolderOps` / `Spawn` seams.
3. Lock waits do not consume the budget — charged only by IO waiting.
4. Writer keeps content — item identity by `origin` (else first unused same spelling) keeping the item node; unmodeled entries pass through; integers beyond 64 bits refused (`preset-write-number-range` — premise 39); floats compared by bits, `PassThrough` non-finite policy (premise 40); P3 rows pinned so each fails without its property.
5. U1 on every path — partial scan, safety net, overlay reads, late merges through one `first_link` test; `path-through-link`.
6. Per-folder listing sequence — `ScanListing.listed_at`; two interleavings tested.
7. Byte-exact stale detection — SHA-256 per source in `stamp`; `prepare` takes no library lock.
8. Unit order — validate before catalog, refs into the catalog unit, folder IO before the service (§9).
9. Carried vs native — `folder_listing_at_launch` + `FolderLoadSet.carried` (premise 44).
10. Backend registry seam — `Backends` in P1, threaded through format / validate / resolve / service.
11. Wording — after D2 removed the two notices that used 「실리지」, no new Korean sentence uses 「싣다 / 실리다」 (U3 · U4 · B22 checked).

Recommended findings: all applied except the built-in id item, where the "record the narrowing" option was taken instead of opening the namespace (B23), and the 10,000-cap item, where the truncation notice was picked (B22). Batched wording fixes: `LaunchPlan` literals = 6 (premise 45); `check` has four IO batches (§6); `settings-schema-outdated` only under `Warn`; the omitted notice also covers `loads.skills == false`; `split_front_matter` described as the library's rule; another backend's schema error does not block (§3-6); root `Cargo.toml` = 10 members, the 11th added with its comment (premise 21); late merges admit only what the scan admits (§6).
