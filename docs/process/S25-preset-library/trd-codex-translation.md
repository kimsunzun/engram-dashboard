# TRD — codex launch-material translation and the codex settings-check port (S25 step 6)

| Item | Value |
|---|---|
| **Status** | **Draft 1 — 2026-11-09.** Not reviewed. Main's decisions are recorded in §9-1. Four conflicts between those decisions and the code or the parent documents are raised in §9-2 and need main's ruling. Three user decisions stay open (§9-3: U1–U3). The library survey for the new `toml` dependency (§9-4) is to be reported to the user before P1. |
| **Canonical ADRs** | ADR-0334 (the port; flag translation in backend `materialize`; no YAML in agent) · ADR-0337 (codex materials: JSON = `developerInstructions` + `extraRoots`; terminal = the 「기본」 priming + the settings `-c` lines; nothing written) · ADR-0341 (folder entries: codex gets only what it lacks) · ADR-0351 (a failed placement write fails the spawn — the claude precedent that #5 extends to codex) · ADR-0316 (the claude counterpart, pattern only). ADR candidates from this step: §8-1. |
| **Line pointers** | HEAD `1e36176d` (branch `v0.5.0/feat/preset-library`). Every code anchor below was re-read at this commit. Paths are under `crates/engram-dashboard-agent/src/` unless stated otherwise. |
| **Measurements** | [`docs/research/codex-launch-materials-measurements-2026-11-09.md`](../../research/codex-launch-materials-measurements-2026-11-09.md) — M1–M7 on `codex-cli 0.163.0` (cited below as M1 … M7). |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 6 (L876), "ADR-0341 in 6" (L878), the step-5 hand-over (L880), "Step 5 landed" (L867–L874), §1-5 (port L272–L280, translation table L307–L314, folder load set L319). |

## 1. Scope and relation to the S25 TRD

**What this step does.** It fills in everything the parent TRD assigns to codex for launch materials, in the agent crate only:
- **the settings-check port, codex half** — the codex implementation of the step-5 port (`backend/codex/settings.rs`), plus the port additions the codex half needs (§4-1). The additions are additive: the claude half keeps its shape and gains one-line answers;
- **the translation** — `backend/codex/launch.rs` with the codex `materialize` override and the real branch of `build_spec`, for both modes;
- **the JSON channel** — `skills/extraRoots/set` on every app-server process, `developerInstructions` from the merged instructions, and the T6 `additionalContext` on a resume whose instructions changed, with the `experimentalApi` opt-in that it needs (M3);
- **the hash plumbing** — the stored "last applied instructions hash" reaches the codex backend (§4-8).

**Nothing plugs real materials in yet.** Every launch stays Noop until step 8. The Noop argv and the Noop app-server handshake must stay byte-identical (`tests/launch_argv_golden.rs`, `codex/mod.rs` `the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit` L3708, `protocol.rs` L1168).

**From the parent step-6 entry (L876) and "ADR-0341 in 6" (L878) — all in scope:**
- JSON: `developerInstructions` · `extraRoots` (including nested presets' skills — M13) · raw `-c` lines (including model and effort) · resume `additionalContext` (T6);
- terminal: the 「기본」 priming by the current path (`-c developer_instructions=`) + the preset raw `-c` lines; preset instructions and skills are not loaded; nothing is written to the working folder;
- the port's codex implementation: `-c` assembly (dotted key · TOML), skill format (P7AC — through the caller's parser; the front-matter serde type is agent's — M72; tests here use a fake parser — M63), skill-name rule (codex JSON only — M24-3), command-line guard and the 8191-character budget (both modes — M8), unknown key = ⚠;
- folder entries (ADR-0341): JSON = the folder's instruction file joins the merged instructions (asset-library composes it; this step only carries the text), the folder's `.agents/skills` · `.codex/skills` become extra roots (no junction, nothing written); terminal = nothing from the folder + the omission notice; the codex folder load set;
- the same-name measurement across extra roots (done — M2) and its fallback policy (U2).

**Handed over by step 5 (L880) — all in scope:** a skill-root kind in the folder load set · a malformed-skill mark that is not `unsure` · a per-version `SchemaRefetch` variant · an `omitted` list for the terminal folder notice · the registry growing to two slots. The step-5 list is incomplete: the codex skill-format method, the agent-owned codex front-matter type and its parser-seam method, and the command-line guard/budget methods are also needed (parent §1-5 L278–L279 says the port holds them). They are added here, additively (§4-1).

**Not in scope:**
- **emitting notices.** Notices live in `LaunchPlan.notices`, which `check` (asset-library, step 7) fills. `materialize` has no notice channel. This step defines the codes and their wording (U3), the `loads()` knowledge and the folder `omitted` list; step 7 produces the notices;
- **recording the hash at the first-turn event** (`manager.rs:1780` `TODO(TRD S25 §5-2 4 (b))`) — stays with step 8 (§4-8, decision #1);
- the real YAML parser golden for the codex skill format (step 7 — M63);
- a manager hook that turns a build-time overflow into `LaunchBlocked` (decision #7);
- T9 (re-injecting instructions after compaction) and the compaction survival of `extraRoots` (§6-2 of the parent; step 8 lane or later);
- the resume `deprecationNotice` (§10 R9 — tracking candidate).

## 2. Premises checked against code

| # | Premise | Verdict | Evidence |
|---|---|---|---|
| 1 | The port as landed lacks the codex skill-format method, the codex front-matter type, its parser method and the command-line methods | **True** | `backend/settings_check.rs:25-76` — methods `backend` … `folder_load_set` only; `FrontMatterParser` (`:134-138`) has `paths` + `subagent`. The one parser impl is the claude test fake (`backend/claude/settings.rs` ~`:896`). |
| 2 | The codex backend does not override `materialize` | **True** | `codex/mod.rs:658` `TODO(TRD S25 §5-2 6)`; trait default `backend/mod.rs:311-322` → `LaunchArtifacts { real: kit.plan.real, ..default }`. `build_spec` (`:791`) and `open_spawn` (`:1076`) ignore `_artifacts`. |
| 3 | The codex backend cannot see the stored instructions hash | **True** | `materialize(command, kit, cwd, fs)`, `build_spec`, `open_spawn` get no profile. `LaunchKit` (`launch.rs:185`) carries only the new `instructions_hash` (`:195`). `profile.rs:181` `last_applied_instructions_hash` is never written (`manager.rs:1780` TODO). The manager re-reads the profile right after `prepare` (`manager.rs:1546` prepare, `:1557` `reread`). |
| 4 | Nothing after argv assembly can fail | **True** | `build_spec` returns `CommandSpec` (infallible). Only `materialize` → `Err(String)` → `PtyError::SpawnFailed` (`manager.rs:1719-1722`) and `precheck_control_endpoint` (`:1691`, before `materialize`) can fail. |
| 5 | `materialize` does not know the spawn mode, the resume id or the control endpoint | **True** | Signature `backend/mod.rs:311-317`. `build_spec` receives `mode`, `resume_session_id`, `control`; `open_spawn` receives `resume_session_id`, `control`. So `materialize` can bound the resume and MCP pieces only from above (§4-6). |
| 6 | The handshake sends no `capabilities` and is pinned that way | **True** | `protocol.rs:338` `InitializeParams { client_info }`; the comment at `:340` explains why (`requestAttestation` stays false); pin `protocol.rs:1168`. Sending `capabilities: {experimentalApi: true}` alone keeps `requestAttestation` at its default false — the reason in `:340` still holds. |
| 7 | Neither `skills/extraRoots/set` nor `additionalContext` exists in our protocol types | **True** | Method list `protocol.rs:214-226`; `TurnStartParams` `:559` = `thread_id · input · client_user_message_id`. |
| 8 | Unknown app-server notifications are dropped | **True (by design) — to be pinned** | The decoder drops unknown methods (`codex/mod.rs:1205` comment; `decoder.rs` header). No test names `thread/readState/changed` or `turn/moderationMetadata` (M3). P3 adds the pin. |
| 9 | Our `thread/resume` already sends `excludeTurns: true` | **True** | `codex/mod.rs:147` (`exclude_turns: Some(true)`). The `deprecationNotice` seen in M4 came from the measurement harness, which did not send it. See R9. |
| 10 | A rejected `turn/start` fails the spawn | **False** | `transport.rs:584-602` `TurnClose::Rejected` / `Unanswered`: the turn is closed, the text goes back to its held slot and the transport halts; the link was already `Ready` at the handshake, so the spawn stays up. See §9-2 C3. |
| 11 | `CMD_UNSAFE_CHARS` is public | **False** | `engram-dashboard-platform/src/shell.rs:52` is a private `const`; only `path_is_cli_safe(&Path)` (`:61`) is public. See §9-2 C1. |
| 12 | Today's priming text contains cmd metacharacters | **True** | `prompts/agent-priming.md` holds two `<` and two `>` (and none of `' " % \`). They are safe today because the priming argument has spaces, so it is wrapped whole in `"…"`. See §9-2 C2. |
| 13 | The terminal priming override is fail-open | **True** | `codex/mod.rs:379` `developer_instructions_override` warns and drops on `' " % \`, a control character other than CR/LF, or the budget; `priming_text` (`:338`) warns and drops on a read failure. |
| 14 | The budget counter is conservative and UTF-16 | **True** | `codex/mod.rs:301` `CMD_LINE_LIMIT_UTF16 = 8191`, `:304` `CMD_WRAPPER_COST`, `:317` `command_line_cost` (+3 per argument). |
| 15 | `toml` is already in `Cargo.lock` | **True** | `toml 1.1.2+spec-1.1.0` (with `toml_parser 1.1.2`, `toml_datetime 1.1.1`, `serde_spanned 1.1.1`, `winnow`) — via `tauri-utils` / `embed-resource`, i.e. only in the shell's graph (`cargo tree -i toml@1.1.2`). The agent and daemon graphs have none. |
| 16 | `codex app-server --stdio` is a private per-process server | **Likely** | Each codex JSON agent starts its own `codex app-server --stdio` child (`transport.rs:1409` `open` → `spawn_piped`); `proxy` / `daemon` are separate subcommands on 0.163.0. A process-wide `skills/extraRoots/set` therefore reaches only that agent. Not measured. |
| 17 | The codex walk's depth limit is "≤ 6 levels" from the root | **Number known, counting not** | Research `docs/research/preset-folder-entry-backend-loading-2026-11-04.md:65`. Whether `root/a/b/c/d/e/SKILL.md` or one more level is the last loaded one is not written down. P1 fixes the count from the codex source at `rust-v0.163.0` before writing the boundary test (§7-1). |

## 3. What the measurements settle (summary)

Details and evidence are in the research file. Used below as:
- **M1** — a junction under an extra root (or a root that is a junction) is followed; codex reports the target path; a skill without a front-matter `name` takes the target folder's name.
- **M2** — same-name skills from two extra roots are both listed, with no error; `$name` then selects neither.
- **M3** — `additionalContext` needs `capabilities.experimentalApi: true` (`-32600` without it); with it, the key becomes the tag of a developer message; the opt-in adds `thread/readState/changed` (certain) and maybe `turn/moderationMetadata` (unknown); a rejected `turn/start` creates nothing.
- **M4** — extra roots are per process and not restored by `thread/resume`; setting them before `thread/start` / `thread/resume` is enough; a resumed thread still "remembers" skills from its old record.
- **M5** — `"` survives `cmd.exe /c codex.cmd` but flips cmd's quote state for later arguments; `% & ^ | < >` and control characters must stay out; an LF silently truncates the line; a wrong value for a known key kills the app-server at startup; unknown keys only warn.
- **M6** — the `-c` parse rule (first `=` · trim · plain dotted split · TOML value · else trimmed text with outer quotes stripped · last wins).
- **M7** — `AGENTS.override.md` wins by existence (any case on NTFS) even when empty or blank, with no fallback.

## 4. Module boundaries and signatures

Layers touched: platform `shell` (one public helper) · agent `backend/settings_check.rs` (additive port types and methods) · agent `backend/codex/{settings,cmdline,launch}.rs` (new) · `backend/codex/{mod,protocol,transport}.rs` · `backend/claude/settings.rs` (answers to the new methods) · `backend/mod.rs` (registry) · `launch.rs` (kit and artifact fields, notice codes) · `manager.rs` (one assignment). The manager and the transport stay backend-neutral (ADR-0004): only `backend/codex` reads the new artifact fields. Each new item carries the code anchor of its ADR (`// ADR-0334` port · `// ADR-0337` codex materials · `// ADR-0341` folder entries).

### 4-1. Port additions — `backend/settings_check.rs` (additive)

```rust
pub trait SettingsCheck: Send + Sync {
    // … the eleven step-5 methods, unchanged …

    /// The format check this backend runs on one library skill in `mode` (the same rules the CLI uses to load or skip
    /// a skill). `file_name` = the file's own name (the check requires exactly `SKILL.md` where the backend does),
    /// `folder_name` = the skill folder's name (the fallback name), `text` = the whole file.
    /// A backend or mode that loads skills without a format check answers `NotChecked`.
    fn skill_format(
        &self,
        file_name: &str,
        folder_name: &str,
        text: &str,
        parser: &dyn FrontMatterParser,
        mode: AgentOutputFormat,
    ) -> SkillFormat;

    /// The command-line arguments one raw settings value of this backend adds (codex: `-c` + `key=value` per line),
    /// each passing this backend's command-line guard. `Err` = a line the command line cannot carry (problem ②).
    /// A backend whose raw settings never reach the command line answers `Ok(vec![])`.
    fn command_line_args(&self, raw: &Value) -> Result<Vec<String>, Reason>;

    /// Whether the launch command line holds all of `line`, counting the pieces only the launch knows (the control
    /// channel, a resume id) at their upper-bound length. `Err` = over the budget or an instructions text the command
    /// line cannot carry (problem ②). A backend with no command-line budget answers `Ok(())`.
    fn command_line_fits(&self, line: &CommandLine<'_>) -> Result<(), Reason>;
}

pub enum SkillFormat {
    NotChecked,
    /// The identity the backend gives the skill.
    Valid { name: String, description: String },
    Invalid(Reason),
}

pub struct CommandLine<'a> {
    pub mode: AgentOutputFormat,
    /// The normalized working folder (codex terminal puts it on the line with `--cd`).
    pub cwd: &'a Path,
    /// The agent's pass-through arguments (`command.extra_args`).
    pub extra_args: &'a [String],
    /// What [`SettingsCheck::command_line_args`] returned for each attached raw value, in attach order.
    pub config_args: &'a [String],
    /// The instructions text this mode puts on the command line (codex terminal: the merged instructions file,
    /// which holds only the 「기본」 priming). `None` = none.
    pub instructions: Option<&'a str>,
}

/// The fields codex reads from a `SKILL.md` front matter, typed the way codex deserializes them (`SkillFrontmatter`
/// in `codex-rs/skills/src/parser.rs` @ rust-v0.156.1 — the same at 0.166.0 per parent §1-5). A value of the wrong
/// type is a parse failure, so the codex repair runs (M72). The exact `Option` / `default` shape is copied from that
/// source in P1 and pinned by the step-7 golden.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SkillFrontMatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<SkillMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SkillMetadata {
    #[serde(rename = "short-description")]
    pub short_description: Option<String>,
}

pub trait FrontMatterParser {
    fn paths(&self, text: &str) -> Result<PathsFrontMatter, FrontMatterError>;
    fn subagent(&self, text: &str) -> Result<SubagentFrontMatter, FrontMatterError>;
    /// New — a `SKILL.md` front matter as codex reads it.
    fn skill(&self, text: &str) -> Result<SkillFrontMatter, FrontMatterError>;
}
```

**Schema refetch** — a new variant (decision #13):

```rust
pub enum SchemaRefetch {
    LatestOnCliVersionChange,
    /// The address names the schema of exactly this CLI version. Fetch it once per version; the same version never
    /// needs a new fetch.
    PerCliVersion,
}
```

**Folder load set** (decision #13). `FolderLoadKind` gains `SkillRoot`; `FolderLoadSet` gains three lists. All have `Default`, so the claude literal sites add `..Default::default()` and nothing else changes:

```rust
pub enum FolderLoadKind { Instructions, Rule, Skill, Agent, Command, /* new */ SkillRoot }

pub struct FolderLoadSet {
    pub loads: Vec<FolderLoad>,
    pub unsure: Vec<FolderUnsure>,
    /// New — skills under a skill root that fail the backend's format check. The backend skips them; this is a mark
    /// for the preview, never a block (Engram passes the root, not the skill). Not `unsure`: `unsure` means "the
    /// backend's finer rules were not predicted", this means "the format check failed".
    pub malformed: Vec<FolderMalformed>,
    /// New — the names the backend gives the skills in `loads` (codex: front-matter `name`, else the folder name),
    /// one row per loaded skill, in `loads` order. Empty for a backend that does not name folder skills (claude).
    pub skill_names: Vec<FolderSkillName>,
    /// New — what another mode of the same backend loads from this folder and this mode does not (codex terminal:
    /// the codex JSON set). The input for the omission notice. Empty when nothing is omitted.
    pub omitted: Vec<FolderLoad>,
}

pub struct FolderMalformed { pub path: PathBuf, pub reason: Reason }
pub struct FolderSkillName { pub path: PathBuf, pub name: String }
```

`FolderLoad.path` doc changes from "as listed" to "as listed — for a `SkillRoot`, the root folder (relative to the entry folder)".

**Claude answers** (`backend/claude/settings.rs`, one line each): `skill_format` → `NotChecked` · `command_line_args` → `Ok(vec![])` (the raw value goes into the merged `settings.json`) · `command_line_fits` → `Ok(())` (no claude budget is claimed — parent L314; T-83). The claude test `FakeParser` gains `skill` as a stub that returns an error ("not used by the claude tests") (decision #9).

### 4-2. The codex implementation — `backend/codex/settings.rs` (new)

`pub(crate) static CODEX_SETTINGS_CHECK: CodexSettingsCheck` and `pub(crate) use settings::CODEX_SETTINGS_CHECK;` in `codex/mod.rs`. The registry becomes `settings_checks() -> [&'static dyn SettingsCheck; 2] { [&claude::CLAUDE_SETTINGS_CHECK, &codex::CODEX_SETTINGS_CHECK] }` (`backend/mod.rs:762`).

| Method | Codex answer |
|---|---|
| `backend` | `"codex"` |
| `schema_source(v)` | `https://github.com/openai/codex/releases/download/rust-v{v}/config-schema.json` (research `settings-asset-validation-spike-2026-10-17.md:16`) · `SchemaRefetch::PerCliVersion` |
| `version_probe` | `console_command("codex", ["--version"])`, no env. Whether `--version` ever prints an update banner is unknown; the parser tolerates extra lines. |
| `parse_version` | The first whitespace-separated word, on any line and at any position, that is a dotted version. `codex-cli 0.163.0` → `0.163.0`. (The claude rule — the first word of a line — would return `None` here.) |
| `raw_to_json(raw)` | §4-2a |
| `merge` | `None` — codex loads each `-c` line on its own; a later line wins key by key, so a type error is always in one preset's own value (step-5 shape). |
| `unknown_keys` | `Warn` |
| `loads(mode)` | JSON: `{instructions: All, skills: true}` · Terminal: `{instructions: BuiltinPrimingOnly, skills: false}` |
| `accepts_skill_name(name, mode)` | JSON: 1–64 characters of `[a-z0-9-]` · Terminal: `true` (M24-3) |
| `folder_entry_blocks` | `false` — codex puts no folder path on its command line (the extra roots travel in JSON) |
| `folder_load_set` | §4-2c |
| `skill_format` | §4-2b (JSON); `NotChecked` in Terminal |
| `command_line_args` / `command_line_fits` | §4-3 |

#### 4-2a. Raw text → JSON (decision #10, M6)

The codex raw value is a JSON array of strings, one `key=value` override each (parent §1-2 L119). For each string, in order:
1. split at the first `=` — none ⇒ `Err` (`codex-config-line-invalid`, args `{index, line}`);
2. trim the key and the value; an empty key ⇒ `Err` (same code);
3. split the key on `.` with no quoting (empty segments allowed, as codex allows `a..b=1`);
4. value = `toml::from_str::<toml::Table>(&format!("_x_ = {value}"))` and take `_x_`. If that fails, the value is the trimmed text with leading and trailing `"` and `'` characters stripped (M6 — the trim set is inferred from two cases, **likely**);
5. insert into the result object along the dotted path, a later line replacing an earlier value at the same path.

A non-array raw value or a non-string element ⇒ `Err` (`codex-config-not-a-list`). Conversion of the TOML value to JSON is written by hand: a TOML datetime becomes its TOML text as a JSON string (serde would emit toml's private datetime struct); a non-finite float becomes its text as a string (the schema then decides).

**Not measured:** a later line that goes through an earlier scalar (`a=1` then `a.b=2`). P1 measures it with `config/read {includeLayers: true}` (no tokens, a temporary `CODEX_HOME` — the M5/M6 method) and mirrors the result; until then the implementation replaces the scalar with a table.

#### 4-2b. Skill format (P7AC · M63 · parent L278)

`skill_format(file_name, folder_name, text, parser, Json)`:
1. `file_name != "SKILL.md"` (exact, case-sensitive) ⇒ `Invalid(codex-skill-file-name)`.
2. A leading U+FEFF is **not** stripped — the separation below then finds no front matter ⇒ `Invalid(codex-skill-no-front-matter)`. (Codex's `trim()` does not strip U+FEFF either.)
3. **Separation** = a hand copy of codex `extract_frontmatter`: `lines()`; the first line `trim() == "---"`; the block runs to the next line whose `trim() == "---"`; no opening line, no closing line, or zero lines in between ⇒ no front matter ⇒ `Invalid(codex-skill-no-front-matter)`. A whitespace-only block goes on to parsing.
4. **Parse** = `parser.skill(block)`. On `Err(original)`: run the codex **repair** (`repair_frontmatter_scalar_fields`, ported in full at the text level) once and call `parser.skill(repaired)`. If that also fails ⇒ `Invalid(codex-skill-front-matter, args {message}, line = the original error's line mapped to the file)` — the **original** error, not the second one. If the first call succeeded, the parser is not called again.
5. **Check** (after deserialization; no repair here): `name` = the front-matter name with whitespace runs collapsed and trimmed, else `folder_name` when empty; `name.chars().count() > 64` ⇒ `Invalid(codex-skill-name-too-long)`. `description` missing or empty after trim ⇒ `Invalid(codex-skill-description-empty)`; no length cap. A whitespace-only `metadata.short-description` counts as absent.
6. ⇒ `Valid { name, description }`.

The `[a-z0-9-]` name rule is **not** part of `skill_format` — it is `accepts_skill_name` (Engram's rule for library skills in codex JSON — M24-3). A folder skill is judged by `skill_format` only (codex loads it whatever its spelling).

#### 4-2c. Folder load set (ADR-0341 · decisions #13, #14)

Input = the listing of one folder entry (`FolderEntry::File` / `UnenteredLink`, paths relative to the folder), the reader closure, the parser, the mode. Codex JSON:
- **Instruction file (M7, #14).** The pick is `AGENTS.override.md` if a top-level entry of that name exists, compared by the OS name rule (Windows: case-insensitive — `agents.override.md` matches), else `AGENTS.md` by the same rule. The override wins even when it is empty or blank: then **nothing** is loaded and there is no fallback (no `loads` row). The pick is a link ⇒ `unsure` (`Link`, kind `Instructions`). Unreadable ⇒ `unsure` (`Unreadable`). Otherwise one `loads` row (`Instructions`).
- **Skill roots.** `.agents/skills` and `.codex/skills` (components compared by the OS name rule). A root is reported as `loads` (`SkillRoot`) when at least one listed entry lies under it — the listing has no directory entries, so an empty root folder is invisible here (it would load nothing anyway; whether the plan passes it is asset-library's existence check). A root that is itself an `UnenteredLink` ⇒ `unsure` (`Link`, kind `SkillRoot`) — codex follows it (M1), the listing did not.
- **Skills under a root** — codex's `skills/extraRoots/set` walk rules: the walk starts at the root itself; a path component below the root that starts with `.` prunes that subtree; the depth limit per premise 17; only a file named exactly `SKILL.md` (case-sensitive string compare, as codex's walker compares names). An `UnenteredLink` below a root ⇒ `unsure` (`Link`, kind `Skill`) — codex follows it (M1). For each `SKILL.md`: read (failure ⇒ `unsure` `Unreadable`) → `skill_format` → `Valid` ⇒ a `loads` row (`Skill`) + a `skill_names` row; `Invalid(reason)` ⇒ a `malformed` row with that reason (codex skips the skill). A `SKILL.md` directly in a root (not in a subfolder) is treated as a skill — "walked from the root itself" (**likely**; P1 confirms in the source and otherwise reports it `unsure`).
- **Preview policy** (step 5, main 2026-11-09): only clear cases are predicted; anything that depends on codex rules not mirrored here is `unsure`. The malformed mark is not `unsure`.
- **Not loaded by codex JSON, by design:** anything else in the folder — including skills outside the two roots (parent §6-3 L1041) and `.codex/config.toml`.

Codex terminal: `loads`, `unsure`, `malformed`, `skill_names` empty; `omitted` = the `loads` rows the JSON computation produces (instruction file, roots, skills). The same function computes both, so the notice and the JSON launch cannot disagree.

### 4-3. Command-line guard and budget — `backend/codex/cmdline.rs` (new) + `platform::shell`

**Platform** (decision #8, conflict C1): `engram-dashboard-platform/src/shell.rs` gains

```rust
/// Whether a text argument reaches a CLI wrapped by [`console_command`] unchanged — `false` when it holds one of
/// `% & ^ | < >` or a control character (an LF silently truncates the rest of the command line — M5).
pub fn text_is_cli_safe(text: &str) -> bool
```

built on the existing private `CMD_UNSAFE_CHARS` (`:52`), so the set lives in one place. `path_is_cli_safe` is unchanged. (`!` stays out of the set — T-84.)

**Codex** (`cmdline.rs`, shared by the port and `materialize`):

```rust
/// `-c` + value for each line of one raw codex value, refusing a line that `text_is_cli_safe` refuses.
/// `"` and `'` are allowed (M5 — but see conflict C2). Lines are first validated by the §4-2a split rule.
pub(super) fn config_args(raw: &Value) -> Result<Vec<String>, Reason>;

/// The codex terminal priming value `developer_instructions='…'` (TOML literal) for `text`, or why it cannot be
/// carried: a char in `' " % \`, a control char other than CR/LF (LF/CR become the two characters `\n`). The rule of
/// today's `developer_instructions_override`, split out pure (the Noop path keeps its warn-and-drop wrapper).
pub(super) fn priming_value(text: &str) -> Result<String, PrimingRefusal>;

/// The upper-bound cost of the whole command line (UTF-16 units, `command_line_cost` rules):
/// terminal = wrapper + `resume <uuid>` (always counted) + `--cd <cwd> -s workspace-write -a on-request`
///            + extra_args + config_args + the MCP attachment at its bound + the priming value;
/// JSON     = wrapper + `app-server --stdio` + extra_args + config_args + the MCP attachment at its bound.
pub(super) fn fits(line: &CommandLine<'_>) -> Result<(), Reason>;
```

**The MCP bound is made structural.** `mcp_attachment` (`codex/mod.rs:453`) returns `Unrepresentable` when the control URL is longer than a new constant `MCP_URL_BUDGET` (128 UTF-16 units; the daemon's URL is `http://127.0.0.1:<port>/mcp`, at most 27). `precheck_control_endpoint` already fails a spawn on `Unrepresentable`, so the bound used by `fits` can never be below the real value. The resume pair is counted in every terminal spawn (a 36-character uuid).

Reason codes: `codex-config-unsafe-char` (args `{index, line}`) · `codex-priming-unsafe-char` (args `{char}`) · `command-line-too-long` (args `{needed, limit}`).

### 4-4. Launch types — `launch.rs`

```rust
pub struct LaunchKit {
    // … unchanged …
    /// New — the hash of the instructions the agent's conversation last received
    /// (`AgentProfile::last_applied_instructions_hash`). `prepare` leaves it `None`; the manager fills it from the
    /// registry re-read right after `prepare` (§4-8). Read only by `backend/codex` (T6).
    pub last_applied_instructions_hash: Option<String>,
}

pub struct LaunchArtifacts {
    // … unchanged fields; `skill_roots` doc widened (below) …
    /// New — codex: the `-c` values (without the flag) from the preset raw settings, in plan order. Both modes.
    pub config_overrides: Vec<String>,
    /// New — codex terminal: the `developer_instructions='…'` value built from the merged instructions file.
    /// `None` = no priming on the line.
    pub instructions_override: Option<String>,
    /// New — codex JSON: the merged instructions text (read from `kit.instructions_file`). `None` = none or blank.
    pub instructions_text: Option<String>,
    /// New — codex JSON: the kit's instructions hash differs from the last applied one (T6).
    pub instructions_changed: bool,
}
```

`skill_roots` doc: "claude: the plugin folder (inside the volatile folder) · codex JSON: the extra skill roots in order — the junction root inside the volatile folder, then the plan's `extra_skill_roots` (outside it)". The `TODO(TRD S25 §5-2 6)` at `launch.rs:179` is removed. Every `LaunchKit` literal (`launch.rs:530` Noop, test helpers at `launch.rs:1145`, `:1374`, `backend/claude/launch.rs:189-210`, `backend/mod.rs:2055`) adds the field.

Why four typed fields and not `args`: step 5 chose typed fields over the untyped `args` (step-5 D5), and the priming value must sit after the MCP attachment while the preset lines sit before it (ADR-0337 decision 5), so one flat list would not carry the order.

### 4-5. Translation — `backend/codex/launch.rs` (new) + the `materialize` override

```rust
// codex/mod.rs
fn materialize(&self, command: &AgentCommand, kit: &LaunchKit, cwd: &Path, fs: &dyn LaunchFs)
    -> Result<LaunchArtifacts, String> {
    launch::materialize(command, kit, cwd, fs, &|p| std::fs::read_to_string(p))
}

// codex/launch.rs
pub(super) fn materialize(
    command: &AgentCommand,
    kit: &LaunchKit,
    cwd: &Path,
    fs: &dyn LaunchFs,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
) -> Result<LaunchArtifacts, String>;
```

The read closure is a seam for the unit tests (the `LaunchFs` port stays write-only — it carries only what the placement uses, M18). The manager's dispatch already confines `fs` to the volatile folder (`backend/mod.rs:816` → `place_in_volatile`).

**Body.** Noop kit (`!kit.plan.real`) ⇒ `LaunchArtifacts::default()`. Real kit — every check comes **before** the first write (the step-5 D6 order):
1. `config_overrides` = `cmdline::config_args` over `kit.plan.raw_settings` in order (defense in depth — `check` ran the same function). `Err` ⇒ `Err`.
2. Instructions text (decision #5): `kit.instructions_file` `Some` ⇒ `read` it. A read error or non-UTF-8 ⇒ `Err` (fail closed — the ADR-0351 rule; a new ADR, §8-1). Blank ⇒ treated as none.
3. **Terminal:** `instructions_override` = `cmdline::priming_value(text)`; a refusal ⇒ per **U1** (§9-3): (a) `Err`; (b) warn and `None`. Then `cmdline::fits(...)` with the real pieces and the two bounds. Over ⇒ per U1 when dropping the priming would make it fit, else `Err`. Skills, extra roots and folders are ignored; nothing is written (P4-FINAL, ADR-0337 decision 4).
4. **JSON:** `instructions_text` = the text; `instructions_changed` = `kit.instructions_hash.is_some() && kit.instructions_hash != kit.last_applied_instructions_hash`. Extra roots: every `kit.plan.extra_skill_roots` path must be absolute and valid UTF-8, else `Err`. Skill links (decision #4): each `kit.skills` name must pass `accepts_skill_name(name, Json)` (defense in depth — `check` blocks it as ②), else `Err`; two skills on the same link path (`platform::fs::same_path`) ⇒ `Err`. `cmdline::fits(...)` (JSON has no priming on the line) — over ⇒ `Err` (decision #7).
5. Writes (JSON, only with ≥ 1 skill): `fs.link_dir(<vol>\engram\skills\<name>, skill.dir)` per skill. A failed link ⇒ `Err` (ADR-0351 rule). `skill_roots` = `[<vol>\engram\skills]` (when ≥ 1 skill) followed by `kit.plan.extra_skill_roots`.

The layout `<vol>\engram\skills\<name>` is the claude one (`backend/claude/launch.rs:28-29`), so the links sit at the same depth and `kits.release` detaches them the same way. Codex names the skill from its front matter or the **target** folder (M1), so the link name only has to be unique. A real kit with no instructions, no skills and no raw settings writes nothing and yields `real: true` with empty lists (the "empty real kit" tests stay valid).

**Residual overflow (decision #7).** `check` (asset-library, step 7) blocks with `Broken` using `command_line_fits` at the upper bounds. `materialize` runs the same function again with the actual values (the two bounds still apply, since it does not know the mode or the endpoint). An overflow found only there — possible only if an input changed between `check` and `materialize` — returns `Err` and becomes `PtyError::SpawnFailed`, **not** `LaunchBlocked`/`Broken`. Step 6 adds no manager hook. This departs from the parent TRD L279 wording ("cuts it off before spawn with the same `Broken`"); §8-1 hands the amendment over.

### 4-6. `build_spec` — the real branch (`codex/mod.rs:791`)

Order (ADR-0337 decision 5; the "last `-c` wins" rule — M6):
1. base args per mode (unchanged);
2. `extra_args` (unchanged);
3. **new, real only:** `-c <v>` for each `artifacts.config_overrides`. A line whose key is `mcp_servers.engram…` or `developer_instructions` gets the same warn as a pass-through (`passthrough_overrides_key`, `:494`) — ours comes later and wins;
4. MCP attachment (unchanged);
5. terminal priming: **real** ⇒ `artifacts.instructions_override` (the control channel's `priming_file` is ignored, as claude ignores it on the real path); **Noop** ⇒ today's `priming_text(control)` + `developer_instructions_override` path, byte-identical.

The model and effort reach codex only through step 3 (R35); there is never a `-m`.

### 4-7. JSON channel — `protocol.rs`, `transport.rs`, `open_spawn`

**Protocol** (`protocol.rs`):

```rust
pub(crate) const SKILLS_EXTRA_ROOTS_SET: &str = "skills/extraRoots/set";

#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParams {
    pub(crate) client_info: ClientInfo,
    /// New — sent only on a spawn whose first turn carries `additionalContext` (decision #2). `None` = the field is
    /// absent, byte-identical to today.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) capabilities: Option<InitializeCapabilities>,
}

#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeCapabilities { pub(crate) experimental_api: bool }

#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsExtraRootsSetParams { pub(crate) extra_roots: Vec<String> }   // response `{}` — ignored

pub(crate) struct TurnStartParams {
    // … unchanged …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) additional_context: Option<BTreeMap<String, AdditionalContext>>,
}

pub(crate) struct AdditionalContext { pub(crate) kind: &'static str /* "application" */, pub(crate) value: String }

pub(crate) const INSTRUCTIONS_CONTEXT_KEY: &str = "engram-instructions";
```

**Open plan** (replaces the bare `ThreadOpen` the transport takes today):

```rust
pub(crate) struct AppServerOpen {
    pub(crate) thread: ThreadOpen,
    /// Sent with `skills/extraRoots/set` before `thread/start` / `thread/resume`. Empty = not sent.
    pub(crate) extra_roots: Vec<String>,
    /// Attached to this incarnation's first `turn/start`. `Some` also turns on `experimentalApi` at `initialize`.
    pub(crate) first_turn_context: Option<String>,
}
```

`CodexAppServerTransport::open(spec, structured, decoder, open: AppServerOpen, sid_sink, link_sink)` (`transport.rs:1409`).

**Handshake** (`transport.rs:1614`), inside the shared `HANDSHAKE_BUDGET` (10 s):
1. `initialize` — `capabilities: Some({experimentalApi: true})` iff `first_turn_context.is_some()`;
2. `initialized` (unchanged);
3. **new:** if `extra_roots` is not empty, `skills/extraRoots/set {extraRoots}` and wait for its response. An error response or a timeout fails the handshake (the link fails → the spawn fails — fail closed, like any handshake error). Sent on **every** process, start and resume alike (M4, decision #4);
4. `thread/start` / `thread/resume` (unchanged).

**First turn.** `State` gains `first_turn_context: Option<String>`, set from the open plan. `take_turn_locked` (`:1984`) attaches it as `{ "engram-instructions": {kind: "application", value} }` to the first `turn/start` it builds. What happens to it after that turn's outcome is conflict **C3** (§9-2) — the draft default, pending main's ruling, is: clear it when that `turn/start` is answered with success; keep it for the next `turn/start` after `Rejected` (M3: a rejected `turn/start` creates nothing) and after `Unanswered` (a duplicate developer message is cheaper than a lost one). Steer (`turn/steer`) never carries it.

**Wording** (decision #3): value = one English preface line, a blank line, then the full new text:
> `The instructions for this agent changed since its last launch. The current instructions follow and take precedence over earlier instructions in this conversation.`

**`open_spawn`** (`codex/mod.rs:1076`) builds the plan with `app_server_open(spec, resume_session_id, control, artifacts)` (replacing `app_server_thread_open`, `:159`):
- **Noop** (`!artifacts.real`) ⇒ `thread = thread_open(spec, resume, priming_text(control))`, no roots, no context — byte-identical to today.
- **Real** ⇒ `thread = thread_open(spec, resume, artifacts.instructions_text.clone())` (the start branch carries it as `developerInstructions`; the resume branch carries none, as today) · `extra_roots` = `artifacts.skill_roots` as strings · `first_turn_context` = `Some(preface + text)` iff `resume_session_id.is_some() && artifacts.instructions_changed && artifacts.instructions_text.is_some()`.

**Notifications.** `thread/readState/changed`, `turn/moderationMetadata` and `skills/changed` are not decoded (dropped as unknown — premise 8). P3 pins that they produce no events and do not disturb the turn state.

### 4-8. Hash plumbing (decision #1)

- **Field:** `LaunchKit.last_applied_instructions_hash` (§4-4). **Why the kit and not the plan:** the plan is what `check` resolves from the library, while the stored hash is runtime profile state written by the launch (parent §1-5 L245); putting it in the plan would make asset-library copy a T6 field and would read it before the incarnation marker. The kit is the per-incarnation material, and the manager re-reads the registry right after `prepare` (`manager.rs:1557`), after `epoch_for_spawn`, so the value is the freshest one and the same one the step-8 recording will compare against. No trait signature changes (`materialize`, `build_spec`, `open_spawn`, `LaunchKitSource` stay as they are).
- **Manager:** one assignment after the re-read: `kit.last_applied_instructions_hash = reread.last_applied_instructions_hash.clone();` (the binding at `:1546` becomes `let mut kit`). Comment anchor `// ADR-0337`.
- **Recording at the first-turn event stays with step 8** (`manager.rs:1780` TODO 4(b)). It is not a one-line change: it needs a separate, incarnation-guarded registry mutate that runs at the first-turn event whether or not the session-id compare-and-replace wrote anything (parent L245, ADR-0337 impact), carried through the latch closure, plus its tests. Consequence until then: the stored hash stays `None`, so every real codex JSON resume would carry `additionalContext`. No live effect, because real kits are plugged in by step 8 — step 8 must land the recording in the same commit (§8-1).

### 4-9. Notice codes (U3)

Defined in `launch.rs` next to `notice_text` (constants + one wording arm each); produced by step 7. Codes and Korean wording are U3 (§9-3). `args` carry only identity (names and paths), as `notice_fingerprint` requires.

## 5. Answer shapes

No wire or bus shape changes. New `Reason` codes (kebab-case, as the existing ones) — all are problem ② (`Broken`) when `check` produces them, and `Err(String)` text when `materialize` hits them again:

| Code | From | `args` |
|---|---|---|
| `codex-config-not-a-list` | `raw_to_json`, `command_line_args` | `{}` |
| `codex-config-line-invalid` | same | `{index, line}` |
| `codex-config-unsafe-char` | `command_line_args` | `{index, line}` |
| `codex-priming-unsafe-char` | `command_line_fits` (terminal) | `{char}` |
| `command-line-too-long` | `command_line_fits` | `{needed, limit}` |
| `codex-skill-file-name` · `codex-skill-no-front-matter` · `codex-skill-front-matter` · `codex-skill-name-too-long` · `codex-skill-description-empty` | `skill_format` | `{message?}`, `line` where known |

The block-window wording for these codes belongs to the client step. The notice codes are U3.

## 6. Concurrency

- **No new locks.** The manager assignment (§4-8) runs on the spawn path with no lock held, between the re-read and `materialize`. `materialize` reads the instructions file and writes links with no lock held (the same place the claude placement writes).
- **Transport.** `first_turn_context` lives in the existing `State` under its mutex; `take_turn_locked` is already called with that lock held. The extra-roots request is one more round trip inside the handshake's single deadline (`HANDSHAKE_BUDGET`, `transport.rs:229`), before `thread/start` / `thread/resume`; it does not add a clock.
- **Process scope.** `skills/extraRoots/set` is process-wide inside one app-server; each agent has its own `--stdio` child (premise 16, likely), so roots do not leak between agents. A resume opens a new process and sets them again (M4).
- **Race between `check` and `materialize`.** The kit is prepared after `check`, from the same plan; the only inputs `materialize` reads that `check` did not see are the merged instructions file (written by `prepare`) and the stored hash. A changed file shows up as a residual overflow or bad char → `SpawnFailed` (decision #7).

## 7. Test plan

QA binding rows (`.claude/skill-bindings/qa.md` 「돌릴 것 표」):
- **Rust source · `Cargo.toml` · `Cargo.lock`** ⇒ full regression.
- **codex lane** (`backend/codex/**`, `backend/mod.rs`) ⇒ `usage_probe_smoke codex_` + `backend_contract --ignored`.
- **claude lane** (`backend/mod.rs`) ⇒ `control_send` + `usage_probe_smoke claude_` (the registry grows; the claude half must not move).
- **Agent lifecycle · transport** (P3 changes the codex handshake and turn start) ⇒ live run on (§7-3). GUI is not needed: real kits cannot reach the app before step 8, so the live run drives the manager directly.

### 7-1. Automated, per piece

**P1 — port codex half** (`cargo test -p engram-dashboard-agent -- --test-threads=4`, `cargo test -p engram-dashboard-platform -- --test-threads=4`)
- `platform::shell`: `text_is_cli_safe` refuses each of `% & ^ | < >`, LF, CR, TAB and another control char; accepts `"` and `'`; `path_is_cli_safe` results unchanged.
- `raw_to_json`: one row per M6 table line (nested table, array, quoted string, bare literal, trimmed `p0.sp = 5`, `[1,` literal, `"s` literal, `'it''s'` literal, first-`=` split, inline table, quoted key split plainly, last wins, missing `=` ⇒ `Err`, empty key ⇒ `Err`, `a..b=1` accepted); non-array and non-string ⇒ `Err`; a datetime and `inf` become strings; the P1 measurement for `a=1` then `a.b=2`.
- `parse_version`: `codex-cli 0.163.0` ⇒ `0.163.0`; a banner line before it; no version ⇒ `None`. `schema_source` URL and `PerCliVersion`. `version_probe` shape (`console_command`). `loads` / `accepts_skill_name` per mode (64 vs 65 characters, uppercase, `_`, empty). `merge` ⇒ `None`. `unknown_keys` ⇒ `Warn`. `folder_entry_blocks` ⇒ `false`.
- `skill_format` with a **fake parser** that counts calls (M63): wrong file name · BOM · no opening `---` · no closing `---` · empty block · first parse OK ⇒ called once · first fails, repaired succeeds ⇒ called twice with the repaired text · both fail ⇒ the **original** error and line · name collapse and folder fallback · 64 / 65 characters · empty description · whitespace short-description · Terminal ⇒ `NotChecked`. Repair golden at the text level (no parser): colon-space values quoted · values starting with `[ { @ \`` quoted · inner `'` doubled · trailing `#` comment kept · block scalars and quoted values untouched.
- `command_line_args` / `fits`: each banned char rejected in a line; `"` and `'` accepted; the budget edge (exactly 8191 fits, 8192 does not) in both modes with the resume and MCP bounds counted; a priming with `'` / `"` / `%` / `\` / a control char ⇒ `codex-priming-unsafe-char`; LF in the priming becomes `\n` and counts two units. `mcp_attachment` refuses a URL over `MCP_URL_BUDGET`.
- `folder_load_set` over a fake listing: override > `AGENTS.md`; lowercase names match (OS rule); empty and blank override ⇒ nothing, no fallback; override as a link ⇒ `unsure`; the two roots; dot-folder below a root pruned while the dotted root itself is walked; the depth boundary (premise 17); `skill.md` ignored; a link under a root ⇒ `unsure` `Link`; a root that is a link ⇒ `unsure` `SkillRoot`; a malformed skill ⇒ `malformed`, not `unsure`, not `loads`; `skill_names` order; Terminal ⇒ only `omitted`, equal to the JSON `loads`.
- Registry (`backend/mod.rs:1989`, `:2017`): two slots, each key = its backend word; `settings_check_for("CODEX")` finds codex. All existing claude settings tests pass unchanged (the claude fake gains the `skill` stub).

**P2 — shared + terminal translation**
- `launch_argv_golden` and `manager/prelaunch.rs` replay: **byte-identical** (Noop).
- `codex::launch::materialize` called **directly** with the memory `LaunchFs` and a fake reader (parent L792 — the dispatch path cannot prove "nothing outside"): Noop ⇒ default; terminal real ⇒ no write at all, `instructions_override` from the file, `config_overrides` in plan order, skills and extra roots ignored; JSON real ⇒ links at `<vol>\engram\skills\<name>`, `skill_roots` = junction root + plan roots in order, `instructions_text`, `instructions_changed` truth table (none/none, same, different, kit `None` with stored `Some`); a read failure ⇒ `Err` before any write; a relative extra root ⇒ `Err`; a name outside `[a-z0-9-]` ⇒ `Err`; two names on one link ⇒ `Err`; a failed `link_dir` ⇒ `Err`; budget overflow ⇒ `Err`; U1 behaviour per the user's choice; an empty real kit writes nothing.
- `build_spec` real: terminal argv = base + pass-through + preset `-c` lines + MCP + priming, in that order; JSON argv = `app-server --stdio` + pass-through + preset `-c` + MCP; the control `priming_file` is not read on the real path; a preset line setting `developer_instructions` is followed by ours; never `-m`. Noop pins (`interactive_args_are_the_measured_ones` `:1353`, `extra_args_follow_our_base_args`, `a_resuming_spawn_still_carries_our_override_after_the_passthrough`, `app_server_extra_args_come_last` `:2016`) unchanged.
- Manager: a fake kit source whose `prepare` returns `None` for the stored hash, and a profile whose field is `Some("h")` ⇒ the kit reaching `materialize` carries `Some("h")` (`probe_spec` / a recording backend in `manager.rs` tests).

**P3 — JSON channel**
- `protocol.rs`: `initialize` without capabilities (the `:1168` pin stays) and with `{"capabilities":{"experimentalApi":true}}`; `skills/extraRoots/set` params `{"extraRoots":[…]}`; `turn/start` with and without `additionalContext` (`{"engram-instructions":{"kind":"application","value":…}}`); the no-identifier envelope stays byte-identical.
- Handshake goldens (the fake app-server already used by the handshake pin, `codex/mod.rs:3708`): start / resume × roots / no roots × context / no context — the request order (`initialize` → `initialized` → [`skills/extraRoots/set`] → `thread/start|resume`); roots sent on resume too; an error response to `skills/extraRoots/set` fails the link. The Noop pin is unchanged.
- First turn: only the first `turn/start` carries the context; the second does not; a steer never does; the C3 rule as ruled by main (rejected / unanswered / accepted).
- Notifications: `thread/readState/changed`, `turn/moderationMetadata`, `skills/changed` produce no output events and leave the turn state as it was.
- `open_spawn` real: start ⇒ `developerInstructions` = the merged text; resume + changed ⇒ no `developerInstructions`, the context = preface + text, capabilities on; resume + unchanged ⇒ no context, no capabilities.

### 7-2. Codex lane

The existing lane (`usage_probe_smoke codex_`, `backend_contract --ignored`) runs after every piece — it covers the terminal PTY spawn with the Noop argv. P3 adds the live run below to the lane (`qa.md` codex row gains the new test file and its command).

### 7-3. Live run (mandatory — real codex, real JSON turns)

New `#[ignore]` file `crates/engram-dashboard-agent/tests/codex_launch_live.rs`, added to the codex lane in `.claude/skill-bindings/qa.md`. It drives a real `AgentManager` with `with_launch_ports` and a test `LaunchKitSource` that writes a real kit into a temporary volatile folder (the `tests/activation.rs` pattern, `:1232`). Real `~/.codex` (auth needed); the kit's raw settings carry `model_reasoning_effort=low` (so the raw `-c` path is exercised too). **Token budget: 2 real model turns.**

| # | Scene | Pass condition |
|---|---|---|
| L1 | codex JSON, Fresh, real kit: instructions file with marker `L1-DEV-<n>`; one library skill `live-lib` (temp folder); one folder root `<tmp>\folder\.agents\skills\live-folder\SKILL.md`. One turn: "Without using a tool, list the skills whose names start with `live-`, then print the marker from your instructions." | The answer names `live-lib` and `live-folder` and prints `L1-DEV-<n>`. The working folder and the folder entry are unchanged (listing before = after). The volatile folder holds only the junction. |
| L2 | Resume the L1 thread with a kit whose instructions carry `L2-DEV-<m>` (stored hash = L1's, set in the test registry). One turn: "Print the marker from your current instructions." | The answer prints `L2-DEV-<m>`. The handshake carried `experimentalApi` and `skills/extraRoots/set` (transport log line), and the turn was not rejected. |
| L3 | Resume again with the **same** hash, no model turn (kill after the link settles). | No `capabilities`, no context (log); the roots are sent again. |
| L4 | codex terminal, real kit with the priming text and one raw line. No model turn. | The TUI comes up (the `backend_contract` readiness probe); the argv carries the preset `-c` line before the MCP and priming `-c`; no new file in the working folder or the volatile folder. |
| L5 | No tokens, temporary `CODEX_HOME` (M5 method): a preset line with an odd number of `"` followed by the priming argument holding `<` and `>`, in pty style. | Only if main keeps `"` allowed (C2): records whether the priming survives. |

Leftovers: L1/L2 create one thread in the real `~/.codex` (as M2/M4 did); the test prints its id and leaves it — deleting the user's codex data is the user's call.

## 8. Implementation units

Sequential — each ends green (build, its tests, full regression, the codex lane). P1 → P2 → P3; P2 and P3 both edit `codex/mod.rs`, and P2 calls P1's `cmdline` and settings functions.

**P1 — the port's codex half.** Rough size: prod +900, tests +700.
- Files: `engram-dashboard-platform/src/shell.rs` (`text_is_cli_safe`) · `backend/settings_check.rs` (§4-1) · `backend/codex/settings.rs` (new) · `backend/codex/cmdline.rs` (new) · `backend/codex/mod.rs` (`mod settings; mod cmdline;` · `pub(crate) use settings::CODEX_SETTINGS_CHECK;` · the consts and `command_line_cost` become `pub(super)` · `developer_instructions_override` split into the pure `priming_value` + the Noop warn wrapper · `mcp_attachment` URL bound) · `backend/mod.rs` (registry `[_; 2]`, two tests) · `backend/claude/settings.rs` (three one-line answers, the `skill` stub, `..Default::default()` on `FolderLoadSet` literals) · agent `Cargo.toml` + `Cargo.lock` (`toml` — §9-4, after the user report).
- Ends green because nothing calls the new methods outside tests, and the Noop path keeps its behaviour (the wrapper keeps warn-and-drop).

**P2 — shared + terminal translation.** Rough size: prod +450, tests +500.
- Files: `backend/codex/launch.rs` (new) · `backend/codex/mod.rs` (`materialize` override, `build_spec` real branch, remove the `:658` TODO) · `launch.rs` (§4-4 fields and docs, the `:179` TODO, notice code constants and arms — U3) · `manager.rs` (§4-8 assignment) · `LaunchKit` literals in tests (`launch.rs`, `backend/claude/launch.rs`, `backend/mod.rs`).
- Ends green because Noop artifacts are the default (golden byte-identical) and the JSON artifacts are not read by `open_spawn` yet.

**P3 — JSON channel.** Rough size: prod +350, tests +600.
- Files: `backend/codex/protocol.rs` · `backend/codex/transport.rs` (`AppServerOpen`, handshake step, first-turn state) · `backend/codex/mod.rs` (`app_server_open`, `open_spawn`) · `tests/codex_launch_live.rs` (new) · `.claude/skill-bindings/qa.md` (codex lane row).
- Ends green because a Noop open plan has no roots and no context (handshake pin unchanged).

After the pieces: `/engram-review code` per piece (or once over P1–P3) · `/engram-qa` with §7-3 · doc pass (§8-1).

### 8-1. Hand-over to the parent TRD (main copies these in)

- **§5-2 step 6** — a "Step 6 landed" block (as step 5's): pieces, decisions §9, review and QA results.
- **§1-5 L279 (command-line guard)** — amend "step 7 measures once more … cuts it off before spawn with the same `Broken`": the residual overflow found in `materialize` is `Err` → `SpawnFailed` (decision #7); the `Broken` comes only from `check`.
- **§1-5 translation table L310** — "per process · measured likely" → measured certain on 0.163.0 (M1 junction followed, M4 per process incl. resume); add "the `experimentalApi` opt-in is sent only on a spawn that carries `additionalContext`" to L309.
- **§1-5 L319 folder load set** — the walk follows links under a root (M1), so a link under a root is `unsure` (`Link`), and the malformed mark, `skill_names` and `omitted` shapes (§4-1).
- **§6-2 L1004** — strike "whether codex's `extraRoots` walk follows a link" (M1: yes) and "same-name across extra roots" (M2), with the U2 policy; keep the folder-root real turn for step 8 (L1 covers one, but not after compaction).
- **§6-3** — new rows: a resumed codex still recalls removed skills from its record (M4 — accepted, same class as research 2026-10-10); same-name skills across roots (U2 outcome); removing every instruction does not reach a resumed conversation (R6).
- **PRD §6 L281** — the measured result (M2) and the U2 policy.
- **Step 7** — produce the notices with the U3 codes; call `command_line_args` / `command_line_fits` / `skill_format` at check time; the real `format.rs` parser implements `FrontMatterParser::skill` with the golden (incl. repair rows); `prepare` leaves `last_applied_instructions_hash` `None`; `release` must detach links at `<vol>\engram\skills\*` for codex too.
- **Step 8** — the first-turn recording of the hash (TODO 4(b)) lands **in the same commit** that plugs real kits; otherwise every real codex JSON resume re-sends `additionalContext`. Real CLI lane: the codex terminal with the real priming source; compaction survival of `extraRoots`.
- **ADR candidates (`/engram-adr`)** — (a) a failed read of the merged instructions file fails the codex spawn (extends ADR-0351; amends ADR-0216's fail-open for the real path); (b) the U1 outcome for the terminal priming (if (a): amends ADR-0216 for the real path); (c) ADR-0337 impact line: `experimentalApi` only with `additionalContext`, extra roots on every process, junction layout confirmed (M1). Not a new decision for (c) — an amendment note.
- **Tracking candidates** — R9 (resume deprecation), R10 (`"` in pass-through args), and T-84 (`!`) now also covers `text_is_cli_safe`.

## 9. Decisions, conflicts, open items

### 9-1. Decisions main made (2026-11-09)

| # | Decision | Backing |
|---|---|---|
| #1 | The stored hash reaches the backend as `LaunchKit.last_applied_instructions_hash`, filled by one manager line from the post-`prepare` re-read. The first-turn recording stays with step 8 (not a one-line change). | §4-8 · premise 3 |
| #2 | `capabilities.experimentalApi: true` only on spawns that carry `additionalContext`; the transport tolerates `thread/readState/changed` and `turn/moderationMetadata`. | M3 (the opt-in changes only that extra notification; `-32600` without it) · premise 6, 8 |
| #3 | `additionalContext` key = `engram-instructions` (shown as `<engram-instructions>…</engram-instructions>`), `kind: "application"`, English one-line preface (§4-7). Rejection rule: **see C3** — main's "no retry; a rejected first `turn/start` fails the spawn as today" rests on a false premise. | M3 · premise 10 |
| #4 | Skills keep the ADR-0337 junction folder, `<vol>\engram\skills\<name>`, passed as one extra root; extra roots are set on `thread/start` **and** on every resume. | M1 (followed) · M4 (process-scoped) |
| #5 | The merged instructions text is read in `materialize`; a failure fails the spawn (fail closed, as ADR-0351). | §4-5 step 2 |
| #7 | Argv overflow: `check` blocks with `Broken` by upper-bound counting (parent L279); a residual overflow at `materialize` → `Err` → `SpawnFailed`; no new manager hook in step 6. **This amends parent L279** (§8-1). | premise 4, 5 · §4-3 MCP bound |
| #8 | `-c` guard: refuse `% & ^ \| < >`, control characters (incl. LF/CR) and non-UTF-8 in every preset `-c` line; allow `"` and `'`. The set comes from `platform::shell` (new `text_is_cli_safe` over the existing private set). **See C1 and C2.** | M5 |
| #9 | Plain names mirroring step 5: `backend/codex/settings.rs`, `CODEX_SETTINGS_CHECK`, registry of two; the claude fake parser gets the new method as a stub. | — |
| #10 | TOML: the `toml` crate for `-c` value parsing (subject to §9-4 and the user report); mirror codex's `-c` parse per M6. | M6 · §9-4 |
| #13 | Folder load set: additive `SkillRoot` kind, a `malformed` list distinct from `unsure`, an `omitted` list for the terminal notice; `SchemaRefetch::PerCliVersion` for codex. Plus `skill_names` (needed by U2 (a) or (b)). | step-5 hand-over |
| #14 | AGENTS pick: the override wins even when empty or blank (nothing loaded, no fallback); names compared by the OS rule. | M7 |
| — | Preview policy from step 5: clear cases only, the rest `unsure`. `folder_entry_blocks` = `false` for codex. | step 5 |
| — | The resume `deprecationNotice` is a risk / tracking candidate, out of scope (R9 — with the premise-9 correction). | M4 |

### 9-2. Conflicts found — need main's ruling (not resolved here)

- **C1 — `CMD_UNSAFE_CHARS` is not public** (premise 11). The brief says it is already `pub`; it is a private `const` at `shell.rs:52`. The draft adds `pub fn text_is_cli_safe(&str)` over it (the set stays in one place, no duplication). The alternative is making the const `pub`. Either meets #8's intent; the draft's choice keeps the control-character rule next to the char set.
- **C2 — `"` allowed in preset lines × the priming argument's own `<` `>`** (premise 12, M5). In the terminal argv the preset `-c` lines come **before** the priming `-c` (ADR-0337 decision 5). Today's priming file holds two `<` and two `>`, which are safe only because the priming argument is wrapped whole in `"…"` and cmd's quote state is balanced when it arrives. A preset line with an **odd** number of `"` (e.g. `model="o3` — or any typo) flips that state (M5), so the priming's `<`/`>` would be read as redirections — a broken launch, in the very mode #8 meant to protect. #8's "refuse metachars in every `-c` argument" cannot apply to the priming argument without blocking today's priming. M5 did not measure this exact combination (the research file says so). Options:
  - (a) refuse a preset line whose number of `"` is odd (keeps `model="o3"`; keeps cmd's state balanced at every argument boundary in both quoting styles — likely, not measured) ★ draft recommendation;
  - (b) refuse `"` in preset lines (`model=o3` and `model='o3'` still work — the old proposal);
  - (c) keep #8 as is and measure first (L5) — a broken launch is the failure mode if the measurement is skipped.
  The same flip from a pass-through `extra_args` argument is pre-existing and unguarded (R10).
- **C3 — "a rejected first `turn/start` fails the spawn as today's first-turn failure does" is false** (premise 10). Today a rejected `turn/start` closes the turn `Rejected` (the text returns to its held slot and the transport halts); `Unanswered` is handled like `Failed`. The link is already `Ready`, so the spawn does not fail. With "no retry", the instructions-changed context would be dropped, and the next accepted turn would still trigger the first-turn event where step 8 records the new hash — the change would be recorded as applied without having reached the model (a silent loss). Options:
  - (a) carry the context on every `turn/start` until one carrying it is answered with success (M3: a rejected `turn/start` creates nothing, so this cannot duplicate) ★ draft default;
  - (b) treat the rejection of the context-carrying `turn/start` as a link failure and tear the agent down (a new failure path in the transport);
  - (c) no retry, and step 8 must record the hash only when the context was accepted (the transport would have to report that to the latch — a new channel).
- **C4 — parent TRD L279 vs #7.** L279 says the launch-step recheck "cuts it off before spawn with the same `Broken`"; #7 makes it `SpawnFailed`. Main asked to state it explicitly: this TRD follows #7 and hands the L279 amendment over (§8-1). Not a code conflict.

### 9-3. User decisions — open

- **U1 — the 「기본」 priming in the codex terminal does not fit on the real path** (#6). On the real path the priming comes from the merged instructions file and is placed last, after the preset `-c` lines (parent L279: the count includes it so a preset line cannot silently push it out — M8). It may not fit (budget) or may hold a char the priming rule refuses (`' " % \`, a control char other than CR/LF). Today's Noop path warns in the log and launches without it.
  - (a) **Block** — `check` returns `Broken` (`codex-priming-unsafe-char` / `command-line-too-long`), the user fixes the preset lines (or the priming text); a residual case at `materialize` fails the spawn. Consistent with M8 "never drop silently" and with ADR-0351's fail-closed rule. Cost: a long preset `-c` list can block every terminal launch of that agent until shortened. ★ **main recommends** (a).
  - (b) **Warn and launch without the priming** (today's behaviour) — the agent starts without the mail and control instructions of 「기본」; only the daemon log says why. A variant (b′) adds a notice so the caller sees it — needs a U3 code and makes `materialize` a notice producer (it has no channel today).
  - Trade-off: (a) may stop a launch the user expected to work; (b) runs an agent that does not know its mail contract, with no visible cause.
- **U2 — same-name skills across extra roots in codex JSON** (#12). Measured (M2): codex lists both, reports no error, and `$name` then selects neither; the model sees both and may read both files. **The existing conflict check (PRD R4 / parent §1-3 5) does not cover this case**: it counts only library skills actually loaded (including codex JSON's nested presets — M13), and folder entries are explicitly not counted (parent §1-3 "Folder entries in resolution"). Two library skills with the same collision key are already blocked there, and codex's name for a junction skill equals Engram's name (front-matter `name`, else the target folder name — M1), so library-vs-library is covered. Not covered: a folder skill vs a library skill, two folder skills (two folder entries, or `.agents/skills` vs `.codex/skills` of one folder), and Engram roots vs codex's own user or repository skill folders.
  - (a) **Block** with a conflict reason — extend the conflict check to folder skills, using the folder load set's `skill_names` (§4-1). Cost: a folder the user does not control can block the launch; the user scopes stay invisible anyway.
  - (b) **Accept + a launch notice naming the duplicates** (folder-vs-library and folder-vs-folder, from `skill_names`) ★ **main recommends** — the agent still sees both, and the notice tells the human why `$name` does not pick one. Shown once per agent (ADR-0341 decision 5).
  - (c) **Accept silently + document** in PRD §6 and parent §6-3 — the cheapest; the human finds out only when `$name` does nothing.
- **U3 — notice codes and wording for codex-terminal omissions** (#11, parent §2-4 8, PRD R37 addendum). Facts only, says the omission is intended, recommends no fix (G28). Proposed (to be confirmed by the user):

  | Code | `args` | Korean wording (one sentence) |
  |---|---|---|
  | `codex-terminal-omitted` | `{instructions: [library paths], skills: [names]}` | 「알림: codex 터미널 모드는 프리셋 지시서와 스킬을 싣지 않습니다(의도된 동작). 빠진 것 — 지시서 <이름> · 스킬 <이름>」 |
  | `codex-terminal-folder-omitted` | `{folder, items: [relative paths]}` | 「알림: codex 터미널 모드는 폴더 항목을 읽지 않습니다(의도된 동작). 빠진 것 — <폴더>: <파일 · 스킬 폴더>」 |
  | `codex-same-name-skills` (only with U2 (b)) | `{name, paths: [...]}` | 「알림: 이름이 같은 스킬이 codex 에 함께 실립니다 — <이름>: <경로> · <경로>. codex 는 둘 다 보이고 `$<이름>` 으로는 어느 것도 고르지 않습니다.」 |

  Alternative: split the first code per kind (`…-instructions-omitted`, `…-skills-omitted`) — two lines instead of one; the PRD R37 draft is one line. With U1 (b′), one more code (`codex-terminal-priming-omitted`).

### 9-4. Library survey — TOML value parsing (for the user report — CLAUDE.md, parent §1-12 0a rule)

Need: parse the value side of a codex `-c` override as a TOML value (inline tables, arrays, basic and literal strings with escapes, integers, floats, booleans, datetimes) exactly as codex does, then convert it to JSON for the schema check (§4-2a).

| Candidate | License · maintenance | Size / graph | Fit | Verdict |
|---|---|---|---|---|
| `toml` 1.1.2 (`toml-rs/toml`) | MIT OR Apache-2.0 · actively maintained (the toml-rs project; spec 1.1.0 support) · MSRV 1.85 | Already in `Cargo.lock` with `toml_parser`, `toml_datetime`, `serde_spanned`, `winnow` (via the shell's Tauri graph) ⇒ **0 new packages**; newly linked into the daemon binary. Features: `default-features = false, features = ["std", "serde", "parse"]` (drops `display` / `toml_writer`). | `toml::from_str::<toml::Table>("_x_ = …")` is the same call shape codex uses (codex-rs depends on `toml`; its override parser wraps the value this way — **likely**, from memory; the source at `rust-v0.163.0` was not re-read here). A recursion limit is on by default (`unbounded` off). | **Recommended** |
| `toml_edit` 0.25.12 | same project and license | Already in the lock; heavier (a format-preserving document model) | Parses values too, but we never edit TOML text; more API than needed | Rejected |
| Hand-rolled value parser | — | 0 dependencies | Must cover escapes, multi-line and literal strings, numbers with `_` / hex / octal / binary, special floats, datetimes, nested inline tables — and drift from codex silently (a value codex accepts would be blocked, or the reverse) | Rejected |

Not checked: whether 1.1.2 is the latest release (offline). A new direct dependency is a T-62 target (license notice regeneration). **Report to the user before P1** with this table.

## 10. Risks

- **R1 C2 unresolved** — if `"` stays allowed without the parity rule and without L5, an odd `"` in a preset line can break the terminal launch through the priming's `<`/`>`.
- **R2 Mirroring codex by hand** — the `-c` parser (M6 trim set: likely), the walk depth (premise 17), `extract_frontmatter` and the repair are copies of codex behaviour; codex updates itself. Golden rows are drawn from codex behaviour (M6 table, parent L278 sources); the real-parser golden runs in step 7. A drift blocks a valid skill or passes a bad one.
- **R3 Hash not recorded until step 8** — every real codex JSON resume would re-send the context; harmless only because real kits arrive with step 8 (§8-1 couples them).
- **R4 `turn/moderationMetadata`** — possibly opt-in gated (M3 unknown). Dropped as unknown either way; the P3 pin covers it.
- **R5 Resumed conversations remember removed skills** (M4) — removing a skill from a preset does not make a resumed codex forget it. Accepted (parent §6-3 row, §8-1).
- **R6 Instructions removed entirely** — kit hash `None` with a stored hash: no context is sent (there is no text), so a resumed conversation keeps the old instructions. Accepted for step 6; recorded in parent §6-3.
- **R7 Same-name skills** (U2) — whatever is chosen, codex user and repository skill folders stay outside Engram's view.
- **R8 Developer instructions vs a preset `developer_instructions` line in JSON mode** — which wins (`thread/start.developerInstructions` or the `-c` config) is not measured. Engram's own instructions travel in `developerInstructions`; a preset that also sets the key may be ignored or may replace it. Recorded; the check gives no warning.
- **R9 Resume deprecation** — codex warns that full-history hydration is deprecated for paginated threads. Our `thread/resume` already sends `excludeTurns: true` (premise 9), so the warning in M4 came from the harness; the remaining risk is upstream changing `thread/resume` semantics. Tracking candidate.
- **R10 Pass-through arguments are not guarded** — an odd `"` or a metachar in `command.extra_args` reaches cmd unchanged (pre-existing; the user's own input). Tracking candidate.
- **R11 Live-run cost and leftovers** — 2 real turns per lane run; each run leaves a thread in the real `~/.codex`.
- **R12 Large test edits in `transport.rs`** (11.9k lines) — the open-plan type replaces `ThreadOpen` in `open`; every call site in its tests changes mechanically.

## 11. Review record

- 2026-11-09 — Draft 1 written (TRD author subagent). Not reviewed. Pending: main's rulings on C1–C4, the user's U1–U3, the user report of §9-4.
