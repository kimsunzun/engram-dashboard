# TRD — codex launch-material translation and the codex settings-check port (S25 step 6)

| Item | Value |
|---|---|
| **Status** | **Draft 4 — 2026-11-09 — review round 2 applied.** Main's decisions are in §9-1; main's rulings on the four conflicts found in draft 1 (C1–C4) are in §9-2; the user's decisions U1–U3 are in §9-3; the `toml` survey (§9-4) was reported to the user with no objection. Drafts 3–4 add the review fixes and the M8/M9 measurements (§11). Draft 1 = `21bc0063`. |
| **Canonical ADRs** | ADR-0334 (the port; flag translation in backend `materialize`; no YAML in agent) · ADR-0337 (codex materials: JSON = `developerInstructions` + `extraRoots`; terminal = the 「기본」 priming + the settings `-c` lines; nothing written) · ADR-0341 (folder entries: codex gets only what it lacks) · ADR-0351 (a failed placement write fails the spawn — the claude precedent that #5 extends to codex) · ADR-0215 (codex instructions = `developer_instructions`, the backend reads the file — its decisions 4 and 5 (terminal fail-open, read fail-open) are overturned for real materials by U1 and #5, and its decision 7 ("taken only at `thread/start`") is amended by M8 — §8-1) · ADR-0316 (the claude counterpart, pattern only). ADR candidates from this step: §8-1. |
| **Line pointers** | HEAD `1e36176d` (branch `v0.5.0/feat/preset-library`). Every code anchor below was re-read at this commit. Paths are under `crates/engram-dashboard-agent/src/` unless stated otherwise. |
| **Measurements** | [`docs/research/codex-launch-materials-measurements-2026-11-09.md`](../../research/codex-launch-materials-measurements-2026-11-09.md) — M1–M9 on `codex-cli 0.163.0` (cited below as M1 … M9; M8–M9 = the second run, mock provider, no model turns). |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 6 (L876), "ADR-0341 in 6" (L878), the step-5 hand-over (L880), "Step 5 landed" (L867–L874), §1-5 (port L272–L280, translation table L307–L314, folder load set L319). |

## 1. Scope and relation to the S25 TRD

**What this step does.** It fills in everything the parent TRD assigns to codex for launch materials, in the agent crate only:
- **the settings-check port, codex half** — the codex implementation of the step-5 port (`backend/codex/settings.rs`), plus the port additions the codex half needs (§4-1). The additions are additive: the claude half keeps its shape and gains one-line answers;
- **the translation** — `backend/codex/launch.rs` with the codex `materialize` override and the real branch of `build_spec`, for both modes;
- **the JSON channel** — `skills/extraRoots/set` on every app-server process, `developerInstructions` from the merged instructions, and the T6 `additionalContext` on a resume whose instructions changed, with the `experimentalApi` opt-in that it needs (M3);
- **the hash plumbing** — the stored "last applied instructions hash" reaches the codex backend (§4-8);
- **a pre-existing fix** (M8) — every `thread/resume` carries `developerInstructions` = the current full instructions text, Noop and real alike, so a resumed JSON thread keeps its instructions across a compaction in the resumed process (§4-7). It is named separately in the P3 commit body.

**Nothing plugs real materials in yet.** Every launch stays Noop until step 8. The Noop argv and the Noop app-server handshake must stay byte-identical (`tests/launch_argv_golden.rs`, `codex/mod.rs` `the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit` L3708, `protocol.rs` L1168) — with one deliberate exception: the Noop `thread/resume` gains `developerInstructions` (the pre-existing fix above), so the resume rows of the handshake pin change on purpose.

**From the parent step-6 entry (L876) and "ADR-0341 in 6" (L878) — all in scope:**
- JSON: `developerInstructions` · `extraRoots` (including nested presets' skills — M13) · raw `-c` lines (including model and effort) · resume `additionalContext` (T6);
- terminal: the 「기본」 priming by the current path (`-c developer_instructions=`) + the preset raw `-c` lines; preset instructions and skills are not loaded; nothing is written to the working folder;
- the port's codex implementation: `-c` assembly (dotted key · TOML), skill format (P7AC — through the caller's parser; the front-matter serde type is agent's — M72; tests here use a fake parser — M63), skill-name rule (codex JSON only — M24-3), command-line guard and the 8191-character budget (both modes — M8), unknown key = ⚠;
- folder entries (ADR-0341): JSON = the folder's instruction file joins the merged instructions (asset-library composes it; this step only carries the text), the folder's `.agents/skills` · `.codex/skills` become extra roots (no junction, nothing written); terminal = nothing from the folder + the omission notice; the codex folder load set;
- the same-name measurement across extra roots (done — M2) and its fallback policy (U2 — decided: launch + a notice naming the duplicates).

**Handed over by step 5 (L880) — all in scope:** a skill-root kind in the folder load set · a malformed-skill mark that is not `unsure` · a per-version `SchemaRefetch` variant · an `omitted` list for the terminal folder notice · the registry growing to two slots. The step-5 list is incomplete: the codex skill-format method, the agent-owned codex front-matter type and its parser-seam method, and the command-line guard/budget methods are also needed (parent §1-5 L278–L279 says the port holds them). They are added here, additively (§4-1).

**Not in scope:**
- **emitting notices.** Notices live in `LaunchPlan.notices`, which `check` (asset-library, step 7) fills. `materialize` has no notice channel. This step defines the codes and their wording (U3 — decided), the `loads()` knowledge, the folder `omitted` list and the inputs for the same-name notice (`skill_names`, `native_skill_roots`); step 7 produces the notices;
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
| 6 | The handshake sends no `capabilities` and is pinned that way | **True** | `protocol.rs:338` `InitializeParams { client_info }`; the comment at `:340` explains why (`requestAttestation` stays false); pin `protocol.rs:1168`. Sending `capabilities: {experimentalApi: true}` alone keeps `requestAttestation` at its default false — the reason in `:340` still holds. The opt-in also enables the server request `currentTime/read` (review round 1); our generic server-request refusal answers it `-32601`. |
| 7 | Neither `skills/extraRoots/set` nor `additionalContext` exists in our protocol types | **True** | Method list `protocol.rs:214-226`; `TurnStartParams` `:559` = `thread_id · input · client_user_message_id`. |
| 8 | Unknown app-server notifications are dropped | **True (by design) — to be pinned** | The decoder drops unknown methods (`codex/mod.rs:1205` comment; `decoder.rs` header). No test names `thread/readState/changed` or `turn/moderationMetadata` (M3). P3 adds the pin. |
| 9 | Our `thread/resume` already sends `excludeTurns: true` | **True** | `codex/mod.rs:147` (`exclude_turns: Some(true)`). The `deprecationNotice` seen in M4 came from the measurement harness, which did not send it. See R9. |
| 10 | A rejected `turn/start` fails the spawn | **False** (above the codex floor) | `transport.rs:584-602` `TurnClose::Rejected` / `Unanswered`: the turn is closed, the text goes back to its held slot and the transport halts; the link was already `Ready` at the handshake, so the spawn stays up. Below the floor (`Floor::Below` — no client id) the entry is removed when the `turn/start` is written (`take_turn_locked` `:1984`), so the text is not returned; the spawn still stays up. The context rule (§4-7) is keyed to the echo, so it holds on both sides of the floor. See §9-2 C3. |
| 11 | `CMD_UNSAFE_CHARS` is public | **False** | `engram-dashboard-platform/src/shell.rs:52` is a private `const`; only `path_is_cli_safe(&Path)` (`:61`) is public. See §9-2 C1. |
| 12 | Today's priming text contains cmd metacharacters | **True** | `prompts/agent-priming.md` holds two `<` and two `>` (and none of `' " % \`). They are safe today because the priming argument has spaces, so it is wrapped whole in `"…"`. See §9-2 C2. |
| 13 | The terminal priming override is fail-open | **True** | `codex/mod.rs:379` `developer_instructions_override` warns and drops on `' " % \`, a control character other than CR/LF, or the budget; `priming_text` (`:338`) warns and drops on a read failure. |
| 14 | The budget counter is an upper bound | **False once `"` or `\` occur** (review round 1 — lead + codex) | `codex/mod.rs:301` `CMD_LINE_LIMIT_UTF16 = 8191`, `:304` `CMD_WRAPPER_COST`, `:317` `command_line_cost` = UTF-16 length + 3 per argument (the two wrapping quotes and the separator). Both quoting rules — std `append_arg` (JSON mode) and portable-pty 0.8.1 `append_quoted` (terminal) — also insert a `\` before every `"` and double the backslashes that precede a `"` (and those before the closing quote); the counter counts neither. It is right today only because its one caller (`developer_instructions_override`) refuses `"` and `\`. Codex's review example: a value of 1700 empty strings (`["","",…]`) counted 5111 and serialized to 8510. `cmdline::fits` counts each `"` and each `\` as 2 units (§4-3). |
| 15 | `toml` is already in `Cargo.lock` | **True** | `toml 1.1.2+spec-1.1.0` (with `toml_parser 1.1.2`, `toml_datetime 1.1.1`, `serde_spanned 1.1.1`, `winnow`) — via `tauri-utils` / `embed-resource`, i.e. only in the shell's graph (`cargo tree -i toml@1.1.2`). The agent and daemon graphs have none. |
| 16 | `skills/extraRoots/set` reaches only the process that received it | **True (measured — M9)** | Each codex JSON agent starts its own `codex app-server --stdio` child (`transport.rs:1409` `open` → `spawn_piped`). M9: two live processes on the same `CODEX_HOME` set different roots; neither sees the other's in `skills/list`, `skills/changed` or the model input. |
| 17 | The codex walk's depth limit is "≤ 6 levels" from the root | **Number known, counting not** | Research `docs/research/preset-folder-entry-backend-loading-2026-11-04.md:65`. Whether `root/a/b/c/d/e/SKILL.md` or one more level is the last loaded one is not written down. P1 fixes the count from the codex source at `rust-v0.163.0` before writing the boundary test (§7-1). |
| 18 | A resumed JSON thread keeps its instructions across a compaction | **False — pre-existing defect** | `thread_open` (`codex/mod.rs:133-157`) puts no `developerInstructions` on the resume branch; the `ThreadResumeParams` doc (`protocol.rs` ~`:421`, "no counterpart … leaving it absent is the decision") is outdated — the field is in the stable schema and accepted (M8). M8: a cold resume replays the stored history (the start-time instructions stay in it), but the first compaction in a resumed process rebuilds the initial context from that process's session settings — with no field, there are **no** instructions after it. With the field, the resumed process's text replaces the old one at its compaction; it never reaches the next turn directly. Fixed in P3 (§4-7). |

## 3. What the measurements settle (summary)

Details and evidence are in the research file. Used below as:
- **M1** — a junction under an extra root (or a root that is a junction) is followed; codex reports the target path; a skill without a front-matter `name` takes the target folder's name.
- **M2** — same-name skills from two extra roots are both listed, with no error; `$name` then selects neither.
- **M3** — `additionalContext` needs `capabilities.experimentalApi: true` (`-32600` without it); with it, the key becomes the tag of a developer message; the opt-in adds `thread/readState/changed` (certain) and maybe `turn/moderationMetadata` (unknown); a rejected `turn/start` creates nothing.
- **M4** — extra roots are per process and not restored by `thread/resume`; setting them before `thread/start` / `thread/resume` is enough; a resumed thread still "remembers" skills from its old record.
- **M5** — `"` survives `cmd.exe /c codex.cmd` but flips cmd's quote state for later arguments; `% & ^ | < >` and control characters must stay out; an LF silently truncates the line; a wrong value for a known key kills the app-server at startup; unknown keys only warn.
- **M6** — the `-c` parse rule (first `=` · trim · plain dotted split · TOML value · else trimmed text with outer quotes stripped · last wins).
- **M7** — `AGENTS.override.md` wins by existence (any case on NTFS) even when empty or blank, with no fallback.
- **M8** — `thread/resume.developerInstructions` is accepted but does not reach the next turn; it is held in the resumed process's session and used only when a compaction there rebuilds the initial context. Without it, that rebuild has no instructions at all. `additionalContext` lives in history only, so every compaction drops it.
- **M9** — `skills/extraRoots/set` is per process: two live app-servers on one `CODEX_HOME` do not see each other's roots.

## 4. Module boundaries and signatures

Layers touched: platform `shell` (one public helper) · agent `backend/settings_check.rs` (additive port types and methods) · agent `backend/codex/{settings,cmdline,launch}.rs` (new) · `backend/codex/{mod,protocol,transport,decoder,usage_probe}.rs` · `backend/claude/{settings,launch}.rs` (answers to the new methods; artifact literals) · `backend/mod.rs` (registry) · `launch.rs` (kit and artifact fields, notice codes) · `manager.rs` (one assignment). The manager and the transport stay backend-neutral (ADR-0004): only `backend/codex` reads the new artifact fields. Each new item carries the code anchor of its ADR (`// ADR-0334` port · `// ADR-0337` codex materials · `// ADR-0341` folder entries).

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

    /// The command-line arguments one raw settings value of this backend adds, exactly as they go on the line (codex:
    /// `-c`, `key=value`, `-c`, `key=value`, … — flag and value alternating), each passing this backend's command-line
    /// guard. `Err` = a line the command line cannot carry (problem ②).
    /// A backend whose raw settings never reach the command line answers `Ok(vec![])`.
    fn command_line_args(&self, raw: &Value) -> Result<Vec<String>, Reason>;

    /// Whether the launch command line holds all of `line`, counting the pieces only the launch knows (the control
    /// channel, a resume id) at their upper-bound length. `Err` = over the budget or an instructions text the command
    /// line cannot carry (problem ②). A backend with no command-line budget answers `Ok(())`.
    fn command_line_fits(&self, line: &CommandLine<'_>) -> Result<(), Reason>;

    /// The skill folders this backend reads by itself in `mode`, besides the roots Engram passes — the input for the
    /// same-name notice (U2). Pure: candidate paths and rules only; the caller stats and lists them. Empty for a
    /// backend or mode that needs no such notice (claude; codex terminal).
    fn native_skill_roots(&self, places: &NativePlaces<'_>, mode: AgentOutputFormat) -> NativeSkillRoots;

    /// The skills this backend loads from one skill root, predicted from a listing of that root (the same walk and
    /// format check as the skill roots inside [`SettingsCheck::folder_load_set`]). Used for the native roots above.
    fn root_skills(
        &self,
        entries: &[FolderEntry],
        read: &dyn Fn(&Path) -> std::io::Result<String>,
        parser: &dyn FrontMatterParser,
    ) -> FolderLoadSet;
}

pub struct NativePlaces<'a> {
    pub home: &'a Path,
    /// The agent's normalized working folder.
    pub cwd: &'a Path,
    /// An environment variable as the agent's process would see it (the profile env over the daemon's).
    pub env: &'a dyn Fn(&str) -> Option<String>,
}

pub struct NativeSkillRoots {
    /// Roots read as they are (codex: `$CODEX_HOME/skills` — default `<home>/.codex/skills` — and
    /// `<home>/.agents/skills`).
    pub fixed: Vec<PathBuf>,
    /// Root names looked for in `cwd` and in each parent up to the project root (codex: `.agents/skills` at every
    /// level, `.codex/skills` where codex has a project config layer).
    pub per_folder: Vec<PathBuf>,
    /// The names whose presence marks the project root (codex default markers); none found = `cwd` only.
    pub project_markers: Vec<String>,
}

pub enum SkillFormat {
    NotChecked,
    /// The identity the backend gives the skill.
    Valid { name: String, description: String },
    Invalid(Reason),
}

pub struct CommandLine<'a> {
    pub mode: AgentOutputFormat,
    /// The command interpreter as the spawn writes it at the start of the line — the resolved `cmd.exe` path
    /// (`%ComSpec%` and the `PATH` lookup; the caller passes the longer of the two — portable-pty writes the resolved
    /// path). The caller resolves it through `platform::shell` (new `console_program_path`); the port stays pure.
    pub console_program: &'a str,
    /// The normalized working folder (codex terminal puts it on the line with `--cd`).
    pub cwd: &'a Path,
    /// The agent's pass-through arguments (`command.extra_args`).
    pub extra_args: &'a [String],
    /// What [`SettingsCheck::command_line_args`] returned for the attached raw values, concatenated in attach order —
    /// the exact arguments (flags included) that go on the line, so the count covers exactly what is placed.
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

**Folder load set** (decision #13). `FolderLoadKind` gains `SkillRoot`; `FolderLoadSet` gains three lists (`malformed`, `skill_names` — also the U2 notice input — and `omitted`). All have `Default`; the claude half builds its sets through `FolderLoadSet::default()` and pushes (there is no `FolderLoadSet { … }` literal outside the type's own file), so the claude code does not change:

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

**Claude answers** (`backend/claude/settings.rs`, one line each): `skill_format` → `NotChecked` · `command_line_args` → `Ok(vec![])` (the raw value goes into the merged `settings.json`) · `command_line_fits` → `Ok(())` (no claude budget is claimed — parent L314; T-83) · `native_skill_roots` → empty · `root_skills` → `FolderLoadSet::default()`. The claude test `FakeParser` gains `skill` as a stub that returns an error ("not used by the claude tests") (decision #9).

### 4-2. The codex implementation — `backend/codex/settings.rs` (new)

`pub(crate) static CODEX_SETTINGS_CHECK: CodexSettingsCheck` and `pub(crate) use settings::CODEX_SETTINGS_CHECK;` in `codex/mod.rs`. The registry becomes `settings_checks() -> [&'static dyn SettingsCheck; 2] { [&claude::CLAUDE_SETTINGS_CHECK, &codex::CODEX_SETTINGS_CHECK] }` (`backend/mod.rs:762`).

| Method | Codex answer |
|---|---|
| `backend` | `"codex"` |
| `schema_source(v)` | `https://github.com/openai/codex/releases/download/rust-v{v}/config-schema.json` (research `settings-asset-validation-spike-2026-10-17.md:16`) · `SchemaRefetch::PerCliVersion` |
| `version_probe` | `console_command("codex", ["--version"])`, no env. Whether `--version` ever prints an update banner is unknown; the parser tolerates extra lines. |
| `parse_version` | The word right after a `codex-cli` word on a line, when it is a dotted version: `codex-cli 0.163.0` → `0.163.0`. Any other line is ignored, so a banner line that holds a version of something else is not taken. No such line ⇒ `None`. (The claude rule — the first word of a line — would return `None` here.) |
| `raw_to_json(raw)` | §4-2a |
| `merge` | `None` — codex loads each `-c` line on its own; a later line wins key by key, so a type error is always in one preset's own value (step-5 shape). |
| `unknown_keys` | `Warn` |
| `loads(mode)` | JSON: `{instructions: All, skills: true}` · Terminal: `{instructions: BuiltinPrimingOnly, skills: false}` |
| `accepts_skill_name(name, mode)` | JSON: 1–64 characters of `[a-z0-9-]` · Terminal: `true` (M24-3) |
| `folder_entry_blocks` | `false` — codex puts no folder path on its command line (the extra roots travel in JSON) |
| `folder_load_set` | §4-2c |
| `skill_format` | §4-2b (JSON); `NotChecked` in Terminal |
| `command_line_args` / `command_line_fits` | §4-3 |
| `native_skill_roots` / `root_skills` | §4-2d (JSON); `native_skill_roots` is empty in Terminal |

#### 4-2a. Raw text → JSON (decision #10, M6)

The codex raw value is a JSON array of strings, one `key=value` override each (parent §1-2 L119). For each string, in order:
1. split at the first `=` — none ⇒ `Err` (`codex-config-line-invalid`, args `{index, line}`);
2. trim the key and the value; an empty key ⇒ `Err` (same code);
3. split the key on `.` with no quoting (empty segments allowed, as codex allows `a..b=1`);
4. value = `toml::from_str::<toml::Table>(&format!("_x_ = {value}"))` and take `_x_`. If that fails, the value is the trimmed text with every leading and trailing `"` and `'` character stripped (`trim_matches(|c| c == '"' || c == '\'')`);
5. the key `use_legacy_landlock` is renamed `features.use_legacy_landlock` (codex's one key alias);
6. insert into the result object along the dotted path, a later line replacing an earlier value at the same path.

**Checked against the codex source** (draft 2): steps 1, 2, 4 and 5 are `CliConfigOverrides::parse_overrides` / `parse_toml_value` / `canonicalize_override_key` in `codex-rs/utils/cli/src/config_override.rs` at tag `rust-v0.163.0` (read 2026-11-09) — `splitn(2, '=')`, `trim()` on both sides, `"Empty key in override"`, the `_x_ = {raw}` wrapper parsed with `toml::from_str::<toml::Table>`, and the `trim_matches` fallback — **certain**. Draft 1 had the fallback from memory and the trim set as "likely". Step 3 (the dotted split) and step 6 (applying the path) are not in that file — they rest on M6 (certain for the cases measured); the applying code was not re-read (**unverified** beyond M6). Codex resolves `toml 0.9.11+spec-1.1.0` (its `Cargo.lock` at the same tag; workspace dependency `toml = "0.9.5"`); our `toml 1.1.2+spec-1.1.0` implements the same TOML spec (1.1.0) — a difference between the two crate versions is possible but not known (R2).

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
- **Instruction file (M7, #14).** The pick is `AGENTS.override.md` if a top-level entry of that name exists, compared by the OS name rule (Windows: case-insensitive — `agents.override.md` matches), else `AGENTS.md` by the same rule. The override wins even when it is empty or blank: then **nothing** is loaded and there is no fallback (no `loads` row). The pick is a link ⇒ `unsure` (`Link`, kind `Instructions`). Unreadable ⇒ `unsure` (`Unreadable`). A picked `AGENTS.md` (no override) that is empty or blank contributes no text either (M7 — codex drops a blank file) ⇒ no `loads` row. Otherwise one `loads` row (`Instructions`).
- **Skill roots.** `.agents/skills` and `.codex/skills` (components compared by the OS name rule). A root is reported as `loads` (`SkillRoot`) when at least one listed entry lies under it — the listing has no directory entries, so an empty root folder is invisible here (it would load nothing anyway; whether the plan passes it is asset-library's existence check). A root that is itself an `UnenteredLink` ⇒ `unsure` (`Link`, kind `SkillRoot`) — codex follows it (M1), the listing did not. The same when `.agents` or `.codex` itself is an `UnenteredLink` (the root lies behind it).
- **Skills under a root** — codex's `skills/extraRoots/set` walk rules: the walk starts at the root itself; a path component below the root that starts with `.` prunes that subtree; the depth limit per premise 17; only a file named exactly `SKILL.md` (case-sensitive string compare, as codex's walker compares names). An `UnenteredLink` below a root ⇒ `unsure` (`Link`, kind `Skill`) — codex follows it (M1). For each `SKILL.md`: read (failure ⇒ `unsure` `Unreadable`) → `skill_format` → `Valid` ⇒ a `loads` row (`Skill`) + a `skill_names` row; `Invalid(reason)` ⇒ a `malformed` row with that reason (codex skips the skill). A `SKILL.md` directly in a root (not in a subfolder) is treated as a skill — "walked from the root itself" (**likely**; P1 confirms in the source and otherwise reports it `unsure`).
- **Preview policy** (step 5, main 2026-11-09): only clear cases are predicted; anything that depends on codex rules not mirrored here is `unsure`. The malformed mark is not `unsure`.
- **Not loaded by codex JSON, by design:** anything else in the folder — including skills outside the two roots (parent §6-3 L1041) and `.codex/config.toml`.

Codex terminal: `loads`, `unsure`, `malformed`, `skill_names` empty; `omitted` = the `loads` rows the JSON computation produces (instruction file, roots, skills). The same function computes both, so the notice and the JSON launch cannot disagree.

#### 4-2d. Codex's own skill folders (U2 — the same-name notice)

**Codex's root list, read in the source** (`codex-rs/ext/skills/src/host_roots.rs` `resolve_skill_roots` at `rust-v0.163.0`, read 2026-11-09 — **certain** for the list; review round 1 lead 3):
- user config layer (`$CODEX_HOME`): `$CODEX_HOME/skills` (scope User), `<home>/.agents/skills` (User), and the system cache root `system_cache_root_dir($CODEX_HOME)` (System — codex's bundled skills);
- system config layer: `<system config folder>/skills` (scope Admin);
- every project config layer: `<layer folder>/skills`, i.e. a project `.codex/skills` (scope Repo);
- plugin skill roots; the extra roots (`skills/extraRoots/set`, scope User);
- `.agents/skills` in every folder from the project root (found by the project-root markers, `default_project_root_markers()` unless configured) down to `cwd` (scope Repo);
- duplicates by path are removed (`dedupe_skill_roots_by_path`), and `loader/host_merge.rs` merges skills with the same canonical `SKILL.md` path (lead 4).

`native_skill_roots(places, Json)` = `fixed: [$CODEX_HOME/skills (default <home>/.codex/skills), <home>/.agents/skills]` · `per_folder: [.agents/skills, .codex/skills]` · `project_markers` = codex's default markers (the value of `default_project_root_markers()` is read in P1; `.git` expected). Terminal ⇒ empty (the terminal loads no Engram skills, so no skill notice). Which `.codex` folders become project config layers (every level or only some, trust) is decided in `codex_config`, which was not read — `.codex/skills` is a candidate at every level; a level codex does not load can only cause an extra notice, never a missed one.

**Not visible to Engram** ("as far as we can see them" — R7): the system cache root (bundled skills; its path rule was not read), the admin layer (the system config folder), plugin roots, and project-root markers configured in the user's codex config.

`root_skills(entries, read, parser)` = the skill walk of §4-2c applied to one root (`loads` = `Skill` rows, `skill_names`, `malformed`, `unsure`). Whether codex walks its own scopes with the same rules as an extra root (dot-folder pruning, depth) is part of the P1 source check — unverified.

**The comparison is step 7's** (asset-library `check`): the names of the library skills it loads (Engram names — equal to codex's names, M1), the folder entries' `skill_names`, and `root_skills` over each existing native root. **Before counting,** canonicalize every `SKILL.md` path (follow links) and keep one entry per canonical path — codex merges the same canonical path (`host_merge.rs`), so a folder entry that is also the working folder or one of its parents, or a library skill whose folder is also under a scanned root, must not raise a false notice (lead 4). A name then seen in two or more places ⇒ one `codex-same-name-skills` notice per name (§4-9). Names are compared as exact strings (how codex matches a `$name` mention is not measured beyond M2's identical names). Library-vs-library stays the existing conflict block (PRD R4 / parent §1-3 5), so it never reaches the notice.

### 4-3. Command-line guard and budget — `backend/codex/cmdline.rs` (new) + `platform::shell`

**Platform** (decision #8, C1 — ruled by main: as drafted): `engram-dashboard-platform/src/shell.rs` gains

```rust
/// Whether a text argument reaches a CLI wrapped by [`console_command`] unchanged — `false` when it holds one of
/// `% & ^ | < >` or a control character (an LF silently truncates the rest of the command line — M5).
pub fn text_is_cli_safe(text: &str) -> bool
```

built on the existing private `CMD_UNSAFE_CHARS` (`:52`), so the set lives in one place. `path_is_cli_safe` is unchanged. (`!` stays out of the set — T-84.)

**Codex** (`cmdline.rs`, shared by the port and `materialize`):

```rust
/// The arguments one raw codex value adds — `-c`, `key=value`, `-c`, `key=value`, … (flag and value alternating),
/// exactly as placed on the line — refusing a line that `text_is_cli_safe` refuses or that holds an odd number of `"`
/// (C2 — main's ruling). An even number of `"` and any `'` are allowed (M5). Lines are first validated by the §4-2a
/// split rule. The port's `command_line_args`, `CommandLine.config_args` and `LaunchArtifacts.config_args` all carry
/// this one shape.
pub(super) fn config_args(raw: &Value) -> Result<Vec<String>, Reason>;

/// The codex terminal priming value `developer_instructions='…'` (TOML literal) for `text`, or why it cannot be
/// carried: a char in `' " % \`, a control char other than CR/LF (LF/CR become the two characters `\n`). The rule of
/// today's `developer_instructions_override`, split out pure (the Noop path keeps its warn-and-drop wrapper).
pub(super) fn priming_value(text: &str) -> Result<String, PrimingRefusal>;

/// The upper-bound cost of the whole command line, in UTF-16 units. Per argument: its UTF-16 length + 1 for every
/// `"` + 1 for every `\` + 3 (wrapping quotes and separator) — i.e. each `"` and each `\` counts 2. This bounds both
/// quoting rules (std `append_arg`, portable-pty `append_quoted`: a `\` before every `"`, backslashes before a `"` or
/// the closing quote doubled) without copying either rule. Pieces:
/// wrapper  = `console_program` + 2 (its quotes) + `/c` + `codex` (each +1 separator)
///            + `SHIM_MARGIN` (see below);
/// terminal = wrapper + `resume <uuid>` (always counted) + `--cd <cwd> -s workspace-write -a on-request`
///            + extra_args + config_args + the MCP attachment at its bound + the priming value;
/// JSON     = wrapper + `app-server --stdio` + extra_args + config_args + the MCP attachment at its bound.
pub(super) fn fits(line: &CommandLine<'_>) -> Result<(), Reason>;
```

The count applies to every piece, including `cwd` and the pass-through `extra_args` (both may hold `\` or `"`). `command_line_cost` (`codex/mod.rs:317`) stays as it is for the Noop priming path, whose value never holds `"` or `\`.

**The program token** (review round 2 — axis NS4). Today's `CMD_WRAPPER_COST` counts `cmd.exe /c codex` as 17 units, but portable-pty writes the **resolved** interpreter path (e.g. `C:\WINDOWS\system32\cmd.exe`, 27 units, quoted when it has spaces). `fits` counts `console_program` (the longer of `%ComSpec%` and the `PATH` `cmd.exe`, resolved by the caller) + 2. L5 (iii) measures the real line.

**The shim margin** (review round 2 — lead NS3; R13). `cmd.exe` runs `codex.cmd`, whose line `"%_prog%" "%dp0%\node_modules\@openai\codex\bin\codex.js" %*` is expanded again; whether the 8191 limit applies a second time to that expanded line is not known. Until it is measured, `fits` reserves `SHIM_MARGIN = 300` units. **Why a fixed 300 and not the computed prefix:** the prefix depends on where node and the npm global folder are installed, which a pure check cannot read (and which codex's auto-update can move); two typical paths (`C:\Program Files\nodejs\node.exe` and `C:\Users\<name>\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js`, quoted) take about 120–160 units, so 300 covers long user names and custom install folders with room to spare, at the cost of 300 of 8191 units. **P1 runs a no-token measurement** (the M5 method: a line just under 8191 that grows past it after the shim expands `%*`) and keeps the margin if the limit applies again, or sets it to 0 if it does not; the result is recorded in the research file. L5 (iii) re-checks it in the live run.

**The MCP bound is made structural.** `mcp_attachment` (`codex/mod.rs:453`) returns `Unrepresentable` when the control URL is longer than a new constant `MCP_URL_BUDGET` (128 UTF-16 units; the daemon's URL is `http://127.0.0.1:<port>/mcp`, at most 27). `precheck_control_endpoint` already fails a spawn on `Unrepresentable`, so the bound used by `fits` can never be below the real value. The resume pair is counted in every terminal spawn (a 36-character uuid).

Reason codes: `codex-config-unsafe-char` (args `{index, line}` — a banned char or an odd `"` count) · `codex-priming-unsafe-char` (args `{char}`) · `command-line-too-long` (args `{needed, limit}`).

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
    /// New — codex: the arguments the preset raw settings add (`-c`, `key=value`, … — `cmdline::config_args`), in plan
    /// order, exactly as placed on the line. Both modes.
    pub config_args: Vec<String>,
    /// New — codex terminal: the `developer_instructions='…'` value built from the merged instructions file.
    /// `None` = no priming on the line.
    pub instructions_override: Option<String>,
    /// New — codex JSON: the merged instructions text (read from `kit.instructions_file`). `None` = none or blank.
    /// Sent as `developerInstructions` on `thread/start` and on every `thread/resume` (M8).
    pub instructions_text: Option<String>,
    /// New — codex JSON: the kit's instructions hash differs from the last applied one (T6).
    pub instructions_changed: bool,
}
```

`skill_roots` doc: "claude: the plugin folder (inside the volatile folder) · codex JSON: the extra skill roots in order — the junction root inside the volatile folder, then the plan's `extra_skill_roots` (outside it)". The `TODO(TRD S25 §5-2 6)` at `launch.rs:179` is removed. Every `LaunchKit` literal (`launch.rs:530` Noop, test helpers at `launch.rs:1145`, `:1374`, `backend/claude/launch.rs:189-210`, `backend/mod.rs:2055`) adds the field. The full `LaunchArtifacts` literals in `backend/claude/launch.rs` (`:99`, `:291`, `:319`) add the four new fields (or `..Default::default()`) in P2, where the fields appear.

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
1. `config_args` = `cmdline::config_args` over `kit.plan.raw_settings`, concatenated in order (defense in depth — `check` ran the same function). `Err` ⇒ `Err`.
2. Instructions text (decision #5): `kit.instructions_file` `Some` ⇒ `read` it. A read error or non-UTF-8 ⇒ `Err` (fail closed — the ADR-0351 rule; a new ADR, §8-1). Blank ⇒ treated as none.
3. **Terminal:** `kit.plan.instructions` must be empty — the codex terminal plan carries no instruction sources and the merged file holds the 「기본」 priming only (`loads` = `BuiltinPrimingOnly`); a non-empty list ⇒ `Err` (defense in depth — lead 7). `instructions_override` = `cmdline::priming_value(text)`; a refusal ⇒ `Err` (U1 (a) — decided: there is no warn-and-launch path on real materials). Then `cmdline::fits(...)` with the real pieces and the two bounds; over ⇒ `Err`. `check` already blocked both cases with `Broken`, so reaching them here is the residual case (decision #7). Skills, extra roots and folders are ignored; nothing is written (P4-FINAL, ADR-0337 decision 4).
4. **JSON:** `instructions_text` = the text; `instructions_changed` = `kit.instructions_hash.is_some() && kit.instructions_hash != kit.last_applied_instructions_hash`. Extra roots: every `kit.plan.extra_skill_roots` path, and the junction root `<vol>\engram\skills` itself, must be absolute and valid UTF-8, else `Err` (they travel as JSON strings). Skill links (decision #4): each `kit.skills` name must pass `accepts_skill_name(name, Json)` (defense in depth — `check` blocks it as ②), else `Err`; two skills on the same link path (`platform::fs::same_path`) ⇒ `Err`. `cmdline::fits(...)` (JSON has no priming on the line) — over ⇒ `Err` (decision #7).
5. Writes (JSON, only with ≥ 1 skill): `fs.link_dir(<vol>\engram\skills\<name>, skill.dir)` per skill. A failed link ⇒ `Err` (ADR-0351 rule). `skill_roots` = `[<vol>\engram\skills]` (when ≥ 1 skill) followed by `kit.plan.extra_skill_roots`.

The layout `<vol>\engram\skills\<name>` is the claude one (`backend/claude/launch.rs:28-29`), so the links sit at the same depth and `kits.release` detaches them the same way. Codex names the skill from its front matter or the **target** folder (M1), so the link name only has to be unique. A real kit with no instructions, no skills and no raw settings writes nothing and yields `real: true` with empty lists (the "empty real kit" tests stay valid).

**Residual overflow (decision #7).** `check` (asset-library, step 7) blocks with `Broken` using `command_line_fits` at the upper bounds. `materialize` runs the same function again with the actual values (the two bounds still apply, since it does not know the mode or the endpoint). An overflow found only there — possible only if an input changed between `check` and `materialize` — returns `Err` and becomes `PtyError::SpawnFailed`, **not** `LaunchBlocked`/`Broken`. Step 6 adds no manager hook. This departs from the parent TRD L279 wording ("cuts it off before spawn with the same `Broken`"); §8-1 hands the amendment over.

### 4-6. `build_spec` — the real branch (`codex/mod.rs:791`)

Order (ADR-0337 decision 5; the "last `-c` wins" rule — M6):
1. base args per mode (unchanged);
2. `extra_args` (unchanged);
3. **new, real only:** `artifacts.config_args` as they are (the flags are already in the list — the same arguments `fits` counted). A line whose key is `mcp_servers.engram…` or `developer_instructions` gets the same warn as a pass-through (`passthrough_overrides_key`, `:494`) — ours comes later and wins;
4. MCP attachment (unchanged);
5. terminal priming: **real** ⇒ `artifacts.instructions_override` (the control channel's `priming_file` is ignored, as claude ignores it on the real path); **Noop** ⇒ today's `priming_text(control)` + `developer_instructions_override` path, byte-identical.

The model and effort reach codex only through step 3 (R35); there is never a `-m`.

### 4-7. JSON channel — `protocol.rs`, `transport.rs`, `open_spawn`

**Protocol** (`protocol.rs`):

```rust
pub(crate) const SKILLS_EXTRA_ROOTS_SET: &str = "skills/extraRoots/set";

#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeParams {
    // … unchanged …
    /// New — the current full instructions text, on every resume (M8 — the pre-existing fix). It does not reach the
    /// next turn; codex keeps it in the resumed process and uses it when a compaction rebuilds the initial context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

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
1. `initialize` — `capabilities: Some({experimentalApi: true})` iff `first_turn_context.is_some()`. The probe's own `initialize` (`backend/codex/usage_probe.rs:115`) sets `capabilities: None` and stays byte-identical;
2. `initialized` (unchanged);
3. **new:** if `extra_roots` is not empty, `skills/extraRoots/set {extraRoots}` and wait for its response. An error response or a timeout fails the handshake (the link fails → the spawn fails — fail closed, like any handshake error). Sent on **every** process, start and resume alike (M4, M9, decision #4). An old codex without the method answers an error and the spawn fails with the generic handshake error (R16);
4. `thread/start` / `thread/resume` — `thread/resume` now carries `developerInstructions` (below), **only when the version read from the `initialize` `userAgent` is ≥ 0.155.0** (review round 2 — lead NS1; the same version reading as `floor_verdict`; unreadable ⇒ omitted). ADR-0215 decision 7 confirms the field exists from 0.155.0; below it, today's omission stays. The schema has no `additionalProperties: false`, so an older codex would most likely ignore the field anyway — the gate is cheap insurance, not a known break (axis observation).
5. **new log line** (`tracing::info!`, next to the existing "codex thread 개시" line): `experimental_api = on|off`, `extra_roots = <count>`, `resume_instructions = <bool>` (after the version gate), `first_turn_context = <bool>` — counts and flags only, no paths or text. L2/L3 read it (lead B2).

**First turn.** `State` gains `first_turn_context: Option<String>` (set from the open plan), `context_turn: Option<u64>` (the seq of the turn that last carried it), `context_rejections: u8` and `instructions_delivered: bool` (starts `true` when the open plan has no context — nothing to deliver — and `false` when it has one). `take_turn_locked` (`:1984`) attaches the context as `{ "engram-instructions": {kind: "application", value} }` to every `turn/start` it builds while the context is held, and records that turn's seq in `context_turn`.

**Clearing is the reader's own step** (review round 2 — axis NS2), independent of `first_turn_sink` (which is one-shot and may be absent or already consumed): when an echo (`userMessage` item) is attributed to our turn by `claims_our_turn` (`transport.rs:3271`) while the active turn's seq equals `context_turn`, the context is cleared and `instructions_delivered` becomes `true`. So:
- `turn/start` accepted and echoed ⇒ cleared, delivered;
- `Rejected` or `Unanswered` ⇒ kept for the next `turn/start` (M3: a rejected `turn/start` creates nothing, so this cannot duplicate);
- accepted but ended with no echo — `Interrupted`, the early-completion branch with no echo (`transport.rs` ~`:3372-3384`, a `turn/completed` held before the `turn/start` response), the oversized-turn-id success (~`:3320`) and the unreadable success (~`:3423`) — ⇒ kept (the model may not have seen it; a second copy is cheaper than a lost one).

**The retry bound** (review round 2 — axis NS3, replacing draft 3's message-text match): after **2 consecutive `Rejected`** `turn/start`s that carried the context, the transport stops attaching it for this incarnation (`first_turn_context` cleared, `instructions_delivered` stays `false`) and logs a warning; the same text goes into the turn's failure line (the `end_turn_if` reason the screen already shows). An accepted turn resets the count. Recording it as the agent's last failure would need a transport → manager channel that does not exist; not added (§9-1). A codex that silently ignores the field instead (drift) is not detectable here (R15).

**The first-turn report carries the delivery** (review round 2 — MUST, codex · lead NS2 · axis NS1). `FirstTurnSink` (`backend/mod.rs:194`) changes from `Fn()` to `Fn(bool)`: the transport calls it at the first echo (as today) with `instructions_delivered` — `true` only when the echoed turn carried the context, or when no context was needed. Without this, after the retry bound drops the context the next accepted, context-free echo would fire the sink, step 8 would record the new hash although the change never reached the model, and a restart before any compaction would see matching hashes and never re-send it. The latch (`session_id_latch.rs` `first_turn_sink`) takes the bool and keeps it for step 8 (`TODO(TRD S25 §5-2 4 (b))`); session-id persistence is unchanged (it still commits on the first echo, whatever the bool). Other callers: the claude backend ignores the sink (`claude/mod.rs:509`); the test call sites (`manager.rs:3696`, `transport.rs` ~`:9970`, ~`:10109`, `codex/mod.rs:3159`) pass a bool.

Steer (`turn/steer`) never carries the context.

**Wording** (decision #3): value = one English preface line, a blank line, then the full new text:
> `The instructions for this agent changed since its last launch. The current instructions follow and take precedence over earlier instructions in this conversation.`

**`open_spawn`** (`codex/mod.rs:1076`) builds the plan with `app_server_open(spec, resume_session_id, control, artifacts)` (replacing `app_server_thread_open`, `:159`):
- **Noop** (`!artifacts.real`) ⇒ `thread = thread_open(spec, resume, priming_text(control))`, no roots, no context. Byte-identical to today for a start; a resume now carries the same text as `developerInstructions` (the pre-existing fix).
- **Real** ⇒ `thread = thread_open(spec, resume, artifacts.instructions_text.clone())` — the start branch carries it as `developerInstructions`, and so does the resume branch (every resume, changed or not — M8; subject to the ≥ 0.155.0 gate in step 4) · `extra_roots` = `artifacts.skill_roots` as strings · `first_turn_context` = `Some(preface + text)` iff `resume_session_id.is_some() && artifacts.instructions_changed && artifacts.instructions_text.is_some()` (a Fresh spawn never carries it, even when `instructions_changed` is true).

`thread_open` (`codex/mod.rs:133`) passes the instructions to both branches; its doc and the `ThreadResumeParams` doc (`protocol.rs` ~`:421`) are rewritten — the field is in the stable schema, accepted, and held for the next compaction (M8).

**Why both channels on a changed resume** (M8): `developerInstructions` never reaches the next turn — only `additionalContext` delivers the change now; `additionalContext` lives in history only and every compaction drops it — only `developerInstructions` brings the instructions back after a compaction in that process (and then only the new text remains, so no duplicate). This keeps ADR-0215 decision 6 (no contradictory text in two slots): both carry the same text.

**Notifications.** `thread/readState/changed`, `turn/moderationMetadata` and `skills/changed` are not decoded. `turn/moderationMetadata` and `skills/changed` are already in `KNOWN_UNTRANSLATED_METHODS` (`decoder.rs:144-217`, observed as Routine); P3 adds `thread/readState/changed` there, so it is Routine, not Drift (axis 2). P3 pins that the three produce no events and do not disturb the turn state.

### 4-8. Hash plumbing (decision #1)

- **Field:** `LaunchKit.last_applied_instructions_hash` (§4-4). **Why the kit and not the plan:** the plan is what `check` resolves from the library, while the stored hash is runtime profile state written by the launch (parent §1-5 L245); putting it in the plan would make asset-library copy a T6 field and would read it before the incarnation marker. The kit is the per-incarnation material, and the manager re-reads the registry right after `prepare` (`manager.rs:1557`), after `epoch_for_spawn`, so the value is the freshest one and the same one the step-8 recording will compare against. No trait signature changes (`materialize`, `build_spec`, `open_spawn`, `LaunchKitSource` stay as they are).
- **Manager:** one assignment after the re-read: `kit.last_applied_instructions_hash = reread.last_applied_instructions_hash.clone();` (the binding at `:1546` becomes `let mut kit`). Comment anchor `// ADR-0337`.
- **Recording at the first-turn event stays with step 8** (`manager.rs:1780` TODO 4(b)). It is not a one-line change: it needs a separate, incarnation-guarded registry mutate that runs at the first-turn event whether or not the session-id compare-and-replace wrote anything (parent L245, ADR-0337 impact), carried through the latch closure, plus its tests. Consequence until then: the stored hash stays `None`, so every real codex JSON resume would carry `additionalContext`. No live effect, because real kits are plugged in by step 8 — step 8 must land the recording in the same commit (§8-1). **Step 8 records only when the first-turn report says delivered** (`FirstTurnSink(bool)`, §4-7 — review round 2 MUST): a first echo with `false` records nothing, so the next launch still sees a differing hash and re-sends the context.

### 4-9. Notice codes (U3 — decided)

Defined in `launch.rs` next to `notice_text` (a constant + one wording arm each, replacing the generic wording for these codes); produced by step 7. `args` carry only identity (names and paths), as `notice_fingerprint` requires; the wording fills the placeholders from `args` (names joined with `, `), passing each substituted name and path through the existing `escape_controls` (outside input — one line, no terminal escapes).

| Code | `args` | Wording (user 2026-11-09, exact) |
|---|---|---|
| `codex-terminal-omitted` | `{instructions: [library paths], skills: [names]}` | 「알림: codex 터미널 모드는 프리셋 지시서와 스킬을 지원하지 않습니다. 적용되지 않은 항목: 지시서 <이름>, 스킬 <이름>」 |
| `codex-terminal-folder-omitted` | `{folder, items: [relative paths]}` | 「알림: codex 터미널 모드는 폴더 항목을 지원하지 않습니다. 적용되지 않은 폴더: <폴더> (<파일, 스킬 폴더>)」 |
| `codex-same-name-skills` | `{name, paths: [...]}` | 「알림: 같은 이름의 스킬이 여러 곳에 있습니다. codex에서 $<이름>으로 부르면 어느 쪽도 선택되지 않습니다. <이름>: <경로>, <경로>」 |

When a list in `codex-terminal-omitted` is empty, its part is left out (「적용되지 않은 항목: 스킬 a, b」). Style rule from the user (2026-11-09): app wording avoids translationese — no 「싣다 / 실리다」.

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

The block-window wording for these codes belongs to the client step. The notice codes are in §4-9.

## 6. Concurrency

- **No new locks.** The manager assignment (§4-8) runs on the spawn path with no lock held, between the re-read and `materialize`. `materialize` reads the instructions file and writes links with no lock held (the same place the claude placement writes).
- **Transport.** `first_turn_context` lives in the existing `State` under its mutex; `take_turn_locked` is already called with that lock held. The extra-roots request is one more round trip inside the handshake's single deadline (`HANDSHAKE_BUDGET`, `transport.rs:229`), before `thread/start` / `thread/resume`; it does not add a clock.
- **Process scope.** `skills/extraRoots/set` is process-wide inside one app-server and does not reach another live process (premise 16 — measured, M9), so roots do not leak between agents, even on one `CODEX_HOME`. A resume opens a new process and sets them again (M4).
- **Race between `check` and `materialize`.** The plan `materialize` sees is the one `check` produced and the manager matched (parent §1-5 (가) 2). What `check` did not see: the merged instructions file (`prepare` writes it after `check` — a library edit in between changes its text), the stored hash, the volatile path, and the values `materialize` takes from the registry re-read rather than the plan — `command.extra_args` (`manager.rs` ~`:1716-1718`, the step-4 (a) TODO) and the canonicalized `cwd` (~`:1561`, `dunce::canonicalize`, which can lengthen a path). A changed input can only show up as a residual guard or budget failure in `materialize` → `SpawnFailed` (decision #7); the launch never runs past a check it failed.

## 7. Test plan

QA binding rows (`.claude/skill-bindings/qa.md` 「돌릴 것 표」):
- **Rust source · `Cargo.toml` · `Cargo.lock`** ⇒ full regression.
- **codex lane** (`backend/codex/**`, `backend/mod.rs`) ⇒ `usage_probe_smoke codex_` + `backend_contract --ignored`.
- **claude lane** (`backend/mod.rs`) ⇒ `control_send` + `usage_probe_smoke claude_` (the registry grows; the claude half must not move).
- **Agent lifecycle · transport** (P3 changes the codex handshake and turn start) ⇒ live run on (§7-3). The live run drives the manager directly (real kits cannot reach the app before step 8); whether the qa row's GUI check is also run is `/engram-qa`'s call (lead B12).

### 7-1. Automated, per piece

**P1 — port codex half** (`cargo test -p engram-dashboard-agent -- --test-threads=4`, `cargo test -p engram-dashboard-platform -- --test-threads=4`)
- `platform::shell`: `text_is_cli_safe` refuses each of `% & ^ | < >`, LF, CR, TAB and another control char; accepts `"` and `'`; `path_is_cli_safe` results unchanged.
- `raw_to_json`: one row per M6 table line (nested table, array, quoted string, bare literal, trimmed `p0.sp = 5`, `[1,` literal, `"s` literal, `'it''s'` literal, first-`=` split, inline table, quoted key split plainly, last wins, missing `=` ⇒ `Err`, empty key ⇒ `Err`, `a..b=1` accepted); non-array and non-string ⇒ `Err`; a datetime and `inf` become strings; `use_legacy_landlock=true` ⇒ `{features: {use_legacy_landlock: true}}`; `p0.q=""s''` ⇒ `"s"` (every outer quote stripped — `trim_matches`); the P1 measurement for `a=1` then `a.b=2`.
- `parse_version`: `codex-cli 0.163.0` ⇒ `0.163.0`; a banner line before it that holds another version (e.g. `node 22.1.0 detected`) is ignored; no `codex-cli` line ⇒ `None`. `schema_source` URL and `PerCliVersion`. `version_probe` shape (`console_command`). `loads` / `accepts_skill_name` per mode (64 vs 65 characters, uppercase, `_`, empty). `merge` ⇒ `None`. `unknown_keys` ⇒ `Warn`. `folder_entry_blocks` ⇒ `false`.
- `skill_format` with a **fake parser** that counts calls (M63): wrong file name · BOM · no opening `---` · no closing `---` · empty block · first parse OK ⇒ called once · first fails, repaired succeeds ⇒ called twice with the repaired text · both fail ⇒ the **original** error and line · name collapse and folder fallback · 64 / 65 characters · empty description · whitespace short-description · Terminal ⇒ `NotChecked`. Repair golden at the text level (no parser): colon-space values quoted · values starting with `[ { @ \`` quoted · inner `'` doubled · trailing `#` comment kept · block scalars and quoted values untouched.
- `command_line_args` / `fits`: each banned char rejected in a line; `'` accepted; an even number of `"` accepted (`model="o3"`), an odd number refused (`model="o3`) — C2; the budget edge (exactly 8191 fits, 8192 does not) in both modes with the resume and MCP bounds counted; a priming with `'` / `"` / `%` / `\` / a control char ⇒ `codex-priming-unsafe-char`; LF in the priming becomes `\n` and counts two units. The wrapper counts `console_program` + 2 and `SHIM_MARGIN` (a test fixes both values). **Upper-bound rows** (MUST, review round 1): a quote-dense value (the 1700-empty-string array, a value of only `"`), a backslash-heavy value (runs of `\` before a `"` and at the end of an argument), a `cwd` with many `\`, an `extra_args` entry with spaces and quotes — for each, `fits`'s count ≥ the real serialized length: std measured by spawning a probe child with `std::process::Command` that prints its own command-line length (`GetCommandLineW`), portable-pty measured the same way through its `CommandBuilder` (or, if that is not reachable from a test, against a test-only copy of 0.8.1 `append_quoted` pinned to the registry source). `mcp_attachment` refuses a URL over `MCP_URL_BUDGET`.
- `folder_load_set` over a fake listing: override > `AGENTS.md`; a blank `AGENTS.md` (no override) ⇒ no `loads` row; `.agents` or `.codex` as an `UnenteredLink` ⇒ `unsure`; lowercase names match (OS rule); empty and blank override ⇒ nothing, no fallback; override as a link ⇒ `unsure`; the two roots; dot-folder below a root pruned while the dotted root itself is walked; the depth boundary (premise 17); `skill.md` ignored; a link under a root ⇒ `unsure` `Link`; a root that is a link ⇒ `unsure` `SkillRoot`; a malformed skill ⇒ `malformed`, not `unsure`, not `loads`; `skill_names` order; Terminal ⇒ only `omitted`, equal to the JSON `loads`.
- P1 shim measurement (no tokens, §4-3): a line just under 8191 that grows past it after `codex.cmd` expands `%*` — keeps `SHIM_MARGIN` at 300 or sets it to 0; recorded in the research file.
- `native_skill_roots`: JSON ⇒ `$CODEX_HOME/skills` from the env closure, else `<home>/.codex/skills`; `<home>/.agents/skills`; `per_folder` = `.agents/skills`, `.codex/skills`; `project_markers` = codex's defaults; Terminal ⇒ empty. `root_skills` over a fake root listing = the §4-2c skill walk (same rows as a folder entry's root).
- Registry (`backend/mod.rs:1989`, `:2017`): two slots, each key = its backend word; `settings_check_for("CODEX")` finds codex. All existing claude settings tests pass unchanged (the claude fake gains the `skill` stub).

**P2 — shared + terminal translation**
- `launch_argv_golden` and `manager/prelaunch.rs` replay: **byte-identical** (Noop).
- `codex::launch::materialize` called **directly** with the memory `LaunchFs` and a fake reader (parent L792 — the dispatch path cannot prove "nothing outside"): Noop ⇒ default; terminal real ⇒ no write at all, `instructions_override` from the file, `config_args` in plan order (flags included), skills and extra roots ignored; JSON real ⇒ links at `<vol>\engram\skills\<name>`, `skill_roots` = junction root + plan roots in order, `instructions_text`, `instructions_changed` truth table (none/none, same, different, kit `None` with stored `Some`); a read failure ⇒ `Err` before any write; a relative extra root ⇒ `Err`; a name outside `[a-z0-9-]` ⇒ `Err`; two names on one link ⇒ `Err`; a failed `link_dir` ⇒ `Err`; budget overflow ⇒ `Err`; U1 behaviour per the user's choice; an empty real kit writes nothing.
- Terminal real kit with a non-empty `kit.plan.instructions` ⇒ `Err` (lead 7). The junction root must be valid UTF-8 (B9).
- U1 (a): a priming the rule refuses, and a terminal line over the budget only because of the priming, both ⇒ `Err` (no warn-and-launch on the real path); the Noop path still warns and drops (its pin unchanged).
- `launch::notice_text`: the three §4-9 codes give the exact user wording (golden strings, including an empty instructions or skills list); every other code keeps the generic wording.
- `build_spec` real: terminal argv = base + pass-through + preset `-c` lines + MCP + priming, in that order; JSON argv = `app-server --stdio` + pass-through + preset `-c` + MCP; the control `priming_file` is not read on the real path; a preset line setting `developer_instructions` is followed by ours; never `-m`. Noop pins (`interactive_args_are_the_measured_ones` `:1353`, `extra_args_follow_our_base_args`, `a_resuming_spawn_still_carries_our_override_after_the_passthrough`, `app_server_extra_args_come_last` `:2016`) unchanged.
- Manager: a fake kit source whose `prepare` returns `None` for the stored hash and hash `"k"` for the kit, and a profile whose field is `Some("h")` ⇒ observed through the **real** codex `materialize` (no recording backend exists): `artifacts.instructions_changed == true`; with the profile field `Some("k")` ⇒ `false` (lead B3 — via `probe_spec`, which already receives the artifacts).

**P3 — JSON channel**
- `protocol.rs`: `initialize` without capabilities (the `:1168` pin stays) and with `{"capabilities":{"experimentalApi":true}}`; the usage probe's `initialize` unchanged (byte-identical); `thread/resume` with and without `developerInstructions`; `skills/extraRoots/set` params `{"extraRoots":[…]}`; `turn/start` with and without `additionalContext` (`{"engram-instructions":{"kind":"application","value":…}}`); the no-identifier envelope stays byte-identical.
- Handshake goldens: the `:3708` pin is a serializer pin, not a server — so the Windows fake app-server (`bake_fake_app_server` `codex/mod.rs:2456`, script `FAKE_APP_SERVER_PS1` `:2591`) is taught `skills/extraRoots/set` (answers `{}`, or an error in an error mode) and logs every method it receives, in order (axis 5). Start / resume × roots / no roots × context / no context — the received order (`initialize` → `initialized` → [`skills/extraRoots/set`] → `thread/start|resume`); roots sent on resume too; an error response to `skills/extraRoots/set` fails the link. The Noop pin: the start rows unchanged; the resume rows now carry `developerInstructions` (deliberate — the pre-existing fix).
- Resume (M8): a real resume request carries `developerInstructions` = the merged text, changed or not; a Noop resume carries the priming text; a resume with no instructions carries none; a `userAgent` below 0.155.0 (or unreadable) ⇒ the field is omitted (lead NS1). `codex/mod.rs:3690` `a_resuming_app_server_handshake_carries_no_instructions_field` is flipped (renamed and inverted — the resume now carries the field; axis NS5).
- The §4-7 log line: opt-in on/off, root count, resume instructions, context (asserted from a captured subscriber).
- First turn (C3 (a), refined): carried until the echo of a turn that carried it; after the echo the next `turn/start` has none; after `Rejected`, `Unanswered`, `Interrupted` before the echo, the early-completion branch with no echo (~`:3372-3384`), the oversized-turn-id success (~`:3320`) and the unreadable success (~`:3423`) the next `turn/start` carries it again; a steer never does; below the floor (no client id) the same, keyed to the echo.
- Clearing is the reader's step (axis NS2): with the sink absent, or already consumed by an earlier echo, an echo of a carrying turn still clears the context.
- Retry bound (axis NS3): two consecutive `Rejected` carrying turns ⇒ the next `turn/start` has no context, one warning logged, the failure line names it; one rejection then an accepted echo ⇒ cleared and delivered; a rejection, an accepted turn, then a rejection ⇒ still attached (the count resets).
- Delivery report (MUST): the sink is called once with `true` when the first echoed turn carried the context, `true` when there was no context, and `false` when the retry bound dropped it before the first echo. **End-to-end row:** rejection ×2 → stop → a context-free turn echoed (sink `false`) → the latch keeps `false` → a restart with the same stored hash (nothing recorded) still sees `instructions_changed` and re-sends the context. The session id is committed in every case (latch tests unchanged apart from the bool).
- Notifications: `thread/readState/changed`, `turn/moderationMetadata`, `skills/changed` produce no output events and leave the turn state as it was; `thread/readState/changed` is observed as Routine, not Drift (`decoder.rs` `KNOWN_UNTRANSLATED_METHODS`).
- `open_spawn` real: start ⇒ `developerInstructions` = the merged text; resume + changed ⇒ `developerInstructions` = the text **and** the context = preface + text, capabilities on; resume + unchanged ⇒ `developerInstructions` = the text, no context, no capabilities; a Fresh spawn with `instructions_changed: true` ⇒ no context, no capabilities (B10).

### 7-2. Codex lane

The existing lane (`usage_probe_smoke codex_`, `backend_contract --ignored`) runs after every piece — it covers the terminal PTY spawn with the Noop argv. P3 adds the live run below to the lane (`qa.md` codex row gains the new test file and its command).

### 7-3. Live run (mandatory — real codex, real JSON turns)

New `#[ignore]` file `crates/engram-dashboard-agent/tests/codex_launch_live.rs`, added to the codex lane in `.claude/skill-bindings/qa.md`. It drives a real `AgentManager` with `with_launch_ports` and a test `LaunchKitSource` that writes a real kit into a temporary volatile folder (the `tests/activation.rs` pattern, `:1232`). The manager is built with a control endpoint (`new_with_control`) so the MCP `-c` attachment is present (B13). Real `~/.codex` (auth needed); the kit's raw settings carry `model_reasoning_effort=low` (so the raw `-c` path is exercised too). **Token budget: 2 real model turns.**

| # | Scene | Pass condition |
|---|---|---|
| L1 | codex JSON, Fresh, real kit: instructions file with marker `L1-DEV-<n>`; one library skill `live-lib` (temp folder); one folder root `<tmp>\folder\.agents\skills\live-folder\SKILL.md`. One turn: "Without using a tool, list the skills whose names start with `live-`, then print the marker from your instructions." | The answer names `live-lib` and `live-folder` and prints `L1-DEV-<n>`. The working folder and the folder entry are unchanged (listing before = after). The volatile folder holds only the merged instructions file and the junction (B1). |
| L2 | Resume the L1 thread with a kit whose instructions are of realistic size (the 「기본」 priming + a few KB) with the marker `L2-DEV-<m>` **at the end** (stored hash = L1's, set in the test registry). One turn: "Print the marker from your current instructions." | The answer prints `L2-DEV-<m>`. The §4-7 log line shows opt-in on, the root count, resume instructions on, context on; the turn was not rejected. The resumed history still hydrates with the opt-in on (the L1 turn is in the replayed events). The echoed user bubble holds only the user's text (the context is not in it). |
| L3 | Resume again with the **same** hash, no model turn (kill after the link settles). | The log line shows opt-in off, context off, resume instructions on (`developerInstructions` sent — M8), and the roots sent again. |
| L4 | codex terminal, real kit with the priming text and one raw line, with a control endpoint (B13). No model turn. | The TUI comes up (the `backend_contract` readiness probe); the argv carries the preset `-c` line before the MCP and priming `-c`; no new file in the working folder or the volatile folder. |
| L5 | No tokens, temporary `CODEX_HOME` (M5 method), pty style: (i) a preset line with an **even** number of `"` (e.g. `model="o3"`, `p0.s="x y"`) followed by the priming argument holding `<` and `>`; (ii) the same with an **odd** count, as the contrast. | (i) the priming and every later argument arrive intact (`config/read` `sessionFlags`); (ii) shows why the odd count is refused. M5 did not test this combination — L5 confirms the C2 rule. (iii) The real `cmd.exe` line: its program token (the resolved interpreter path) and the shim line — a command line just under 8191 that grows past it after `codex.cmd` expands `%*` — re-checks P1's shim result and the `console_program` count (§4-3, R13). |

Leftovers: L1/L2 create one thread in the real `~/.codex` (as M2/M4 did); the test prints its id and leaves it — deleting the user's codex data is the user's call.

## 8. Implementation units

Sequential — each ends green (build, its tests, full regression, the codex lane). P1 → P2 → P3; P2 and P3 both edit `codex/mod.rs`, and P2 calls P1's `cmdline` and settings functions.

**P1 — the port's codex half.** Rough size: prod +950, tests +800.
- Files: `engram-dashboard-platform/src/shell.rs` (`text_is_cli_safe`, `console_program_path`) · `backend/settings_check.rs` (§4-1) · `backend/codex/settings.rs` (new) · `backend/codex/cmdline.rs` (new) · `backend/codex/mod.rs` (`mod settings; mod cmdline;` · `pub(crate) use settings::CODEX_SETTINGS_CHECK;` · the consts and `command_line_cost` become `pub(super)` · `developer_instructions_override` split into the pure `priming_value` + the Noop warn wrapper · `mcp_attachment` URL bound) · `backend/mod.rs` (registry `[_; 2]`, two tests) · `backend/claude/settings.rs` (the one-line answers to the new methods and the `skill` stub — no `FolderLoadSet` literal changes) · agent `Cargo.toml` + `Cargo.lock` (`toml` — §9-4, after the user report).
- Ends green because nothing calls the new methods outside tests, and the Noop path keeps its behaviour (the wrapper keeps warn-and-drop).

**P2 — shared + terminal translation.** Rough size: prod +450, tests +500.
- Files: `backend/codex/launch.rs` (new) · `backend/codex/mod.rs` (`materialize` override, `build_spec` real branch, remove the `:658` TODO) · `launch.rs` (§4-4 fields and docs, the `:179` TODO, the three notice codes and their wording arms — §4-9) · `manager.rs` (§4-8 assignment) · `LaunchKit` literals in tests (`launch.rs`, `backend/claude/launch.rs`, `backend/mod.rs`) · the full `LaunchArtifacts` literals in `backend/claude/launch.rs` (`:99`, `:291`, `:319` — the new fields appear in P2, so their literals change here, not in P3 as the review listed).
- Ends green because Noop artifacts are the default (golden byte-identical) and the JSON artifacts are not read by `open_spawn` yet.

**P3 — JSON channel.** Rough size: prod +350, tests +600.
- Files: `backend/mod.rs` (`FirstTurnSink` → `Fn(bool)`) · `session_id_latch.rs` (the sink takes and keeps the bool) · `manager.rs` (test call site `:3696`) · `backend/codex/protocol.rs` (`InitializeParams.capabilities`, `ThreadResumeParams.developer_instructions` and its doc, extra-roots and context types) · `backend/codex/transport.rs` (`AppServerOpen`, handshake step, log line, first-turn state and its clearing on the echo, the context-rejection rule) · `backend/codex/mod.rs` (`thread_open` both branches, `app_server_open`, `open_spawn`, the fake app-server script and its method log) · `backend/codex/decoder.rs` (`thread/readState/changed` in `KNOWN_UNTRANSLATED_METHODS`) · `backend/codex/usage_probe.rs` (`InitializeParams` literal → `capabilities: None`) · `tests/codex_launch_live.rs` (new) · `.claude/skill-bindings/qa.md` (codex lane row).
- Ends green because a Noop open plan has no roots and no context; the only Noop handshake change is the resume `developerInstructions`, whose pin rows are updated in the same piece. **The commit body names the pre-existing fix separately** (M8 — resumed JSON threads lost their instructions at the first compaction in a resumed process).

After the pieces: `/engram-review code` per piece (or once over P1–P3) · `/engram-qa` with §7-3 · doc pass (§8-1).

### 8-1. Hand-over to the parent TRD (main copies these in)

- **§5-2 step 6** — a "Step 6 landed" block (as step 5's): pieces, decisions §9, review and QA results.
- **§1-5 L279 (command-line guard)** — amend "step 7 measures once more … cuts it off before spawn with the same `Broken`": the residual overflow found in `materialize` is `Err` → `SpawnFailed` (decision #7); the `Broken` comes only from `check`.
- **§1-5 translation table L310** — "per process · measured likely" → measured certain on 0.163.0 (M1 junction followed, M4 per process incl. resume); add "the `experimentalApi` opt-in is sent only on a spawn that carries `additionalContext`" to L309.
- **§1-5 L319 folder load set** — the walk follows links under a root (M1), so a link under a root is `unsure` (`Link`), and the malformed mark, `skill_names` and `omitted` shapes (§4-1).
- **§6-2 L1004** — strike "whether codex's `extraRoots` walk follows a link" (M1: yes) and "same-name across extra roots" (M2), with the U2 policy (launch + `codex-same-name-skills` notice); keep the folder-root real turn for step 8 (L1 covers one, but not after compaction).
- **§6-3** — new rows: a resumed codex still recalls removed skills from its record (M4 — accepted, same class as research 2026-10-10); same-name skills across roots — launched with a notice naming them (U2 (b), user 2026-11-09), library-vs-library stays the conflict block; removing every instruction does not reach a resumed conversation (R6).
- **PRD §6 L281** — the measured result (M2) and the U2 policy (launch + a notice naming the duplicates; R57 addendum candidate). **PRD R37 addendum** — replace the draft wording with the §4-9 wording (user 2026-11-09).
- **Step 7** — produce the notices with the §4-9 codes, including the same-name comparison (§4-2d: library names + folder `skill_names` + `root_skills` over the existing native roots, listed with the bounded folder listing; canonicalize every `SKILL.md` path and drop identical ones before counting places — lead 4); the codex terminal plan carries no instruction sources and its merged file is the priming only (`materialize` refuses otherwise — lead 7); block a terminal priming that does not fit with `Broken` (U1 (a)); call `command_line_args` / `command_line_fits` / `skill_format` at check time; the real `format.rs` parser implements `FrontMatterParser::skill` with the golden (incl. repair rows); `prepare` leaves `last_applied_instructions_hash` `None`; `release` must detach links at `<vol>\engram\skills\*` for codex too.
- **Step 8** — the first-turn recording of the hash (TODO 4(b)) lands **in the same commit** that plugs real kits; otherwise every real codex JSON resume re-sends `additionalContext`. It records on the first echo **only when the first-turn report is `true`** (`FirstTurnSink(bool)`, §4-7 — the latch keeps the bool); a `false` report records nothing, so a later launch re-sends the context. The hash mutate stays separate from the session-id commit, which is unchanged. Real CLI lane: the codex terminal with the real priming source; compaction survival of `extraRoots` on a real server (M8 saw the skills message rebuilt after a mock compaction — likely survives; not measured on a real server).
- **T9 (parent §2-2) premise** — T9 says compaction discards the developer message. M8 (mock provider, manual compaction, 0.163.0) shows a compaction **in the process that started the thread** keeps the start instructions (V2), and with this step's resume fix a compaction in a resumed process keeps them too. What compaction still drops is `additionalContext` and anything sent only in history. Main to re-word T9 (still unmeasured: automatic compaction, a real server's remote compaction item).
- **ADR candidates (`/engram-adr`)** — (a) a failed read of the merged instructions file fails the codex spawn (extends ADR-0351; amends ADR-0215 decision 5 — read fail-open — for real materials); (b) U1 (a) — a 「기본」 priming that the codex terminal command line cannot carry blocks the launch (amends ADR-0215 decision 4 — terminal warn-and-drop — for real materials); (c) ADR-0337 impact line: `experimentalApi` only with `additionalContext`, extra roots on every process (M4, M9), junction layout confirmed (M1) — an amendment note, not a new decision; (d) U2 (b) — same-name codex skills across roots launch with a notice (answers the PRD §6 row); (e) every `thread/resume` carries `developerInstructions` (M8 — amends ADR-0215 decision 7, "taken only at `thread/start`", which M8 shows is held for the next compaction). Draft 2 named ADR-0216 for (a)/(b); the fail-open rules overturned are ADR-0215's (lead 6).
- **QA binding (note only — the user decides)** — run the codex lane whenever the installed codex CLI version changes, since several rules here mirror codex by hand and a silent drift (R15) is otherwise invisible (axis 1).
- **Tracking candidates** — R9 (resume deprecation), R10 (`"` in pass-through args), and T-84 (`!`) now also covers `text_is_cli_safe`.

## 9. Decisions, conflicts, open items

### 9-1. Decisions main made (2026-11-09)

| # | Decision | Backing |
|---|---|---|
| #1 | The stored hash reaches the backend as `LaunchKit.last_applied_instructions_hash`, filled by one manager line from the post-`prepare` re-read. The first-turn recording stays with step 8 (not a one-line change). | §4-8 · premise 3 |
| #2 | `capabilities.experimentalApi: true` only on spawns that carry `additionalContext`; the transport tolerates `thread/readState/changed` and `turn/moderationMetadata`. | M3 (`-32600` without it; in the measured flow the opt-in added only `thread/readState/changed` — a flow observation, not a schema guarantee; it also enables the server request `currentTime/read`, refused `-32601`) · premise 6, 8 |
| #3 | `additionalContext` key = `engram-instructions` (shown as `<engram-instructions>…</engram-instructions>`), `kind: "application"`, English one-line preface (§4-7). Rejection rule = C3 (a), refined: the context stays attached until the echo of a turn that carried it (draft 1's "no retry; a rejected first `turn/start` fails the spawn" rested on a false premise — premise 10). | M3 · premise 10 · C3 |
| #4 | Skills keep the ADR-0337 junction folder, `<vol>\engram\skills\<name>`, passed as one extra root; extra roots are set on `thread/start` **and** on every resume. | M1 (followed) · M4 (process-scoped) |
| #5 | The merged instructions text is read in `materialize`; a failure fails the spawn (fail closed, as ADR-0351). | §4-5 step 2 |
| #7 | Argv overflow: `check` blocks with `Broken` by upper-bound counting (parent L279); a residual overflow at `materialize` → `Err` → `SpawnFailed`; no new manager hook in step 6. **This amends parent L279** (§8-1). | premise 4, 5 · §4-3 MCP bound |
| #8 | `-c` guard: refuse `% & ^ \| < >`, control characters (incl. LF/CR), non-UTF-8 **and an odd number of `"`** in every preset `-c` line; allow an even number of `"` and any `'`. The set comes from `platform::shell` (new `text_is_cli_safe` over the existing private set). | M5 · C1 · C2 (the odd-`"` rule; the exact combination is unmeasured — L5 confirms) |
| #9 | Plain names mirroring step 5: `backend/codex/settings.rs`, `CODEX_SETTINGS_CHECK`, registry of two; the claude fake parser gets the new method as a stub. | — |
| #10 | TOML: `toml` 1.1.2 (`default-features = false, features = ["std", "serde", "parse"]`) for `-c` value parsing — reported to the user 2026-11-09, no objection (§9-4); mirror codex's `-c` parse per M6 and the codex source (§4-2a). | M6 · §4-2a · §9-4 |
| #13 | Folder load set: additive `SkillRoot` kind, a `malformed` list distinct from `unsure`, an `omitted` list for the terminal notice; `SchemaRefetch::PerCliVersion` for codex. Plus `skill_names`, `native_skill_roots` and `root_skills` — the inputs of the U2 notice (§4-2d). | step-5 hand-over · U2 |
| #14 | AGENTS pick: the override wins even when empty or blank (nothing loaded, no fallback); names compared by the OS rule. | M7 |
| — | Preview policy from step 5: clear cases only, the rest `unsure`. `folder_entry_blocks` = `false` for codex. | step 5 |
| #17 | The command-line count's wrapper = the resolved `cmd.exe` path + 2 (axis NS4) + a fixed `SHIM_MARGIN` of 300 units until P1's measurement keeps or removes it (lead NS3). | §4-3 · R13 |
| — | The resume `deprecationNotice` is a risk / tracking candidate, out of scope (R9 — with the premise-9 correction). | M4 |
| #15 | Keep `additionalContext` on the first turn for changed instructions, **and** send `developerInstructions` = the current full text on **every** `thread/resume` (changed or not, Noop and real) when the codex version is ≥ 0.155.0 (lead NS1) — a pre-existing fix, named separately in the P3 commit. Amends ADR-0215 decision 7 (ADR candidate (e)). | M8 · premise 18 |
| #16 | The held context is cleared by the reader when an echo of a turn that carried it arrives (C3 (a), refined; its own step, independent of the one-shot sink — axis NS2). After 2 consecutive `Rejected` carrying turns it stops for the incarnation (axis NS3 — a count, not a message match), with a warning in the log and the turn's failure line, not in the agent's last failure (no transport → manager channel; not added). `FirstTurnSink` becomes `Fn(bool)`: the first-turn report says whether the instructions were delivered, and step 8 records the hash only when `true`; session-id persistence unchanged (review round 2 MUST). | review rounds 1–2 (lead 5, axis 1, codex MUST, axis NS2–NS3) |

### 9-2. Conflicts found in draft 1 — ruled by main (2026-11-09)

- **C1 — `CMD_UNSAFE_CHARS` is not public** (premise 11). The brief said it was already `pub`; it is a private `const` at `shell.rs:52`. **Ruling:** `pub fn text_is_cli_safe(&str)` over the private set in `platform::shell`, as drafted (§4-3) — the set stays in one place, and the control-character rule sits next to it.
- **C2 — `"` in preset lines × the priming argument's own `<` `>`** (premise 12, M5). In the terminal argv the preset `-c` lines come **before** the priming `-c` (ADR-0337 decision 5). Today's priming file holds two `<` and two `>`, which are safe only because the priming argument is wrapped whole in `"…"` and cmd's quote state is balanced when it arrives. A preset line with an **odd** number of `"` flips that state (M5), so the priming's `<`/`>` would become redirections — a broken launch. #8's metachar ban cannot apply to the priming argument without blocking today's priming. **Ruling: (a)** — refuse a preset `-c` line whose number of `"` is odd, on top of the #8 set (§4-3; `codex-config-unsafe-char`). An even count keeps cmd's quote state balanced at every argument boundary in both quoting styles — **likely**: M5 did not test this exact combination, so live scene L5 confirms it (§7-3). Rejected: banning `"` (would refuse `model="o3"`); keeping #8 unchanged and measuring first (a broken launch if the measurement is skipped). The same flip from a pass-through `extra_args` argument stays pre-existing and unguarded (R10).
- **C3 — "a rejected first `turn/start` fails the spawn as today's first-turn failure does" was false** (premise 10). A rejected `turn/start` closes the turn `Rejected` (the text returns to its held slot and the transport halts); `Unanswered` is handled like `Failed`; the link is already `Ready`, so the spawn does not fail. With "no retry", the context would be dropped while the next accepted turn still triggered step 8's first-turn recording — the change recorded as applied without having reached the model. **Ruling: (a)** — keep the `additionalContext` until a `turn/start` carrying it is accepted (M3: a rejected `turn/start` creates nothing, so this cannot duplicate) (§4-7). **Refined in review round 1** (lead 5): "accepted" = the user-message echo of a turn that carried it, not the `turn/start` success — an accepted turn that ends before its echo keeps the context. **Refined in review round 2:** clearing is the reader's own step (not tied to the one-shot sink); the retry stops after 2 consecutive rejections; and the first-turn report carries whether the instructions were delivered, so step 8 never records a hash the model did not receive. Rejected: tearing the agent down on that rejection (a new transport failure path); no retry plus a delivered-or-not report to the latch (a new channel).
- **C4 — parent TRD L279 vs #7.** L279 says the launch-step recheck "cuts it off before spawn with the same `Broken`"; #7 makes a residual overflow at `materialize` `SpawnFailed`. **Ruling:** follow #7; the L279 amendment is handed over to the parent TRD (§8-1). Not a code conflict.

### 9-3. User decisions — decided (2026-11-09)

- **U1 — the 「기본」 priming that the codex terminal command line cannot carry** (#6) — over the budget, or a char the priming rule refuses (`' " % \`, a control char other than CR/LF). **Decided: (a) block** (user 「띄우지 말아」). `check` blocks the launch with `Broken` (`codex-priming-unsafe-char` / `command-line-too-long`); a residual case at `materialize` fails the spawn (`SpawnFailed`). There is no warn-and-launch path on real materials (§4-5 step 3); the Noop path keeps today's warn-and-drop until it is withdrawn. Consistent with M8 "never drop silently" and ADR-0351. Known cost: a long preset `-c` list can block every terminal launch of that agent until it is shortened. Rejected: (b) warn and launch without the priming (the agent would not know its mail and control contract, with no visible cause) and its notice variant (b′).
- **U2 — same-name skills across skill roots in codex JSON** (#12). Measured (M2): codex lists both, reports no error, and `$name` then selects neither. **Decided: (b) launch + a notice naming the duplicates** (user 「같은 이름이 여러곳있으면 띄우되 알려.」) — code `codex-same-name-skills` (§4-9), produced by step 7 from the inputs in §4-2d. Covered: folder-vs-library, folder-vs-folder (two folder entries, or `.agents/skills` vs `.codex/skills` of one folder), and Engram roots vs codex's own skill folders as far as Engram can see them (`$CODEX_HOME/skills`, `<home>/.agents/skills`, and `.agents/skills` / `.codex/skills` from the project root down to the working folder — §4-2d; the root list was read in the source, `host_roots.rs` at `rust-v0.163.0` — certain; which `.codex` folders codex loads as project layers is not read). **Library-vs-library stays the existing conflict block** (PRD R4 / parent §1-3 5 — it already counts every library skill a codex JSON agent loads, including nested presets, M13; codex names a junction skill the same way Engram does, M1), so it never reaches the notice. M2 measured two extra roots only; that codex treats a user-scope skill and an extra-root skill of the same name the same way rests on research 2026-10-20 row 5 (list all, no priority — 0.156.1). Shown once per agent (ADR-0341 decision 5). Rejected: (a) block (a folder the user does not control could block the launch); (c) accept silently.
- **U3 — notice codes and wording** (#11, parent §2-4 8, PRD R37 addendum). **Decided** (user 「그렇게 하던지」 after rewording, 2026-11-09): the three codes and the exact wording in §4-9. Facts only, no fix recommended (G28). The user's style rule for app wording: no translationese (no 「싣다 / 실리다」). Draft 1's wording and its split-per-kind alternative are withdrawn.

### 9-4. Library survey — TOML value parsing (reported to the user 2026-11-09 — no objection; CLAUDE.md, parent §1-12 0a rule)

Need: parse the value side of a codex `-c` override as a TOML value (inline tables, arrays, basic and literal strings with escapes, integers, floats, booleans, datetimes) exactly as codex does, then convert it to JSON for the schema check (§4-2a).

| Candidate | License · maintenance | Size / graph | Fit | Verdict |
|---|---|---|---|---|
| `toml` 1.1.2 (`toml-rs/toml`) | MIT OR Apache-2.0 · actively maintained (the toml-rs project; spec 1.1.0 support) · MSRV 1.85 | Already in `Cargo.lock` with `toml_parser`, `toml_datetime`, `serde_spanned`, `winnow` (via the shell's Tauri graph) ⇒ **0 new packages**; newly linked into the daemon binary. Features: `default-features = false, features = ["std", "serde", "parse"]` (drops `display` / `toml_writer`). | `toml::from_str::<toml::Table>("_x_ = …")` is the call codex makes (`codex-rs/utils/cli/src/config_override.rs` `parse_toml_value` at `rust-v0.163.0` — read 2026-11-09, **certain**; draft 1 had this from memory). Codex resolves `toml 0.9.11+spec-1.1.0` (workspace `toml = "0.9.5"`); 1.1.2 implements the same spec 1.1.0. A recursion limit is on by default (`unbounded` off). | **Chosen** |
| `toml_edit` 0.25.12 | same project and license | Already in the lock; heavier (a format-preserving document model) | Parses values too, but we never edit TOML text; more API than needed | Rejected |
| Hand-rolled value parser | — | 0 dependencies | Must cover escapes, multi-line and literal strings, numbers with `_` / hex / octal / binary, special floats, datetimes, nested inline tables — and drift from codex silently (a value codex accepts would be blocked, or the reverse) | Rejected |

Not checked: whether 1.1.2 is the latest release. A new direct dependency is a T-62 target (license notice regeneration). **Reported to the user 2026-11-09 with this table — no objection.** Alternative kept in view: `toml` 0.9.12 (also in our lock) would match codex's major line; not chosen because both implement spec 1.1.0 and the report named 1.1.2.

## 10. Risks

- **R1 The odd-`"` rule is likely, not measured** (C2) — an even count of `"` keeping cmd's quote state balanced in both quoting styles is inferred from M5; L5 confirms it. If L5 fails, the fallback is to refuse `"` in preset lines (C2's rejected option).
- **R2 Mirroring codex by hand** — the `-c` parser (split, trim and fallback checked against the codex source; the dotted-path application only against M6; `toml` 1.1.2 vs codex's 0.9.11 — same spec, crate differences unknown), the native skill roots (§4-2d — the list read in the source, certain; the walk rules of those scopes and which `.codex` folders are project layers not read), the walk depth (premise 17), `extract_frontmatter` and the repair are copies of codex behaviour; codex updates itself. Golden rows are drawn from codex behaviour (M6 table, parent L278 sources); the real-parser golden runs in step 7. A drift blocks a valid skill or passes a bad one.
- **R3 Hash not recorded until step 8** — every real codex JSON resume would re-send the context; harmless only because real kits arrive with step 8 (§8-1 couples them).
- **R4 `turn/moderationMetadata`** — possibly opt-in gated (M3 unknown). Dropped as unknown either way; the P3 pin covers it.
- **R5 Resumed conversations remember removed skills** (M4) — removing a skill from a preset does not make a resumed codex forget it. Accepted (parent §6-3 row, §8-1).
- **R6 Instructions removed entirely** — kit hash `None` with a stored hash: no context is sent (there is no text) and the resume carries no `developerInstructions` field (#15), so the resumed conversation keeps the old instructions in its history; the removal reaches the model only at the next compaction in that process, which rebuilds the context with no instructions (M8). The same holds for a Noop resume whose priming file cannot be read. Accepted for step 6; recorded in parent §6-3.
- **R7 Same-name skills** (U2 (b)) — the notice sees codex's own skill folders only as far as §4-2d lists them; a configured project-root marker, codex's bundled system skills (system cache root), the admin layer (system config folder) and plugin-added roots stay invisible. The notice is a hint, not a guarantee; `$name` still selects neither.
- **R8 Developer instructions vs a preset `developer_instructions` line in JSON mode** — which wins (`thread/start.developerInstructions` or the `-c` config) is not measured. Engram's own instructions travel in `developerInstructions`; a preset that also sets the key may be ignored or may replace it. Recorded; the check gives no warning.
- **R9 Resume deprecation** — codex warns that full-history hydration is deprecated for paginated threads. Our `thread/resume` already sends `excludeTurns: true` (premise 9), so the warning in M4 came from the harness; the remaining risk is upstream changing `thread/resume` semantics. Tracking candidate.
- **R10 Pass-through arguments are not guarded** — an odd `"` or a metachar in `command.extra_args` reaches cmd unchanged (pre-existing; the user's own input). Tracking candidate.
- **R11 Live-run cost and leftovers** — 2 real turns per lane run; each run leaves a thread in the real `~/.codex`.
- **R12 Large test edits in `transport.rs`** (11.9k lines) — the open-plan type replaces `ThreadOpen` in `open`; every call site in its tests changes mechanically.
- **R13 The `codex.cmd` shim line** — whether cmd applies 8191 again after `%*` expansion is unverified. Until P1's no-token measurement, `fits` reserves `SHIM_MARGIN = 300` units (§4-3); P1 keeps it or sets it to 0, and L5 (iii) re-checks.
- **R14 Cross-preset object constraints** (lead 8) — `merge` is `None`, so constraints that span presets (e.g. an `mcp_servers.x` table split over two presets, a required field in one and the rest in another) are not checked on the merged value. Rare; codex then exits 1 at startup, which shows as a spawn failure, not a `Broken` block.
- **R15 Silent drift** (axis 1) — a codex that starts ignoring `additionalContext`, `developerInstructions` on resume or an extra root without an error is not detected by the transport; only the live run would show it. Suggested: run the codex lane on every CLI version change (§8-1 note).
- **R16 An old codex without `skills/extraRoots/set`** (axis 7) — answers an error; the handshake fails with the generic handshake error, which does not name the missing method. Recorded; the minimum codex version is not raised here.
- **R17 Resumed instructions are invisible until a compaction** (M8) — on an unchanged resume the model works from the start-time text in its history; the resend only matters after a compaction in that process. Intended.

## 11. Review record

- 2026-11-09 — Draft 4: `/engram-review trd` round 2 (lead PASS, axis PASS, codex FIX) applied — the MUST (`FirstTurnSink(bool)`: the first-turn report carries whether the instructions were delivered; step 8 records only on `true`) and the recommended items (axis NS2 reader-side clearing, NS3 retry bound by count, NS4 resolved interpreter path, NS5 the resume test flipped; lead NS1 version gate ≥ 0.155.0, NS3 shim margin 300 with a P1 measurement) plus the batched wording. Review pending.
- 2026-11-09 — Draft 3: `/engram-review trd deep` round 1 (lead FIX, risk-axis PASS, codex FIX) applied — the MUST (the counter is not an upper bound with `"`/`\` → `fits` counts each as 2; premise 14 corrected; serializer-checked edge rows; the shim line recorded and measured in L5) and the recommended items (lead 2–8, B1–B13; axis 1, 2, 4, 5, 7). M8/M9 added: every `thread/resume` carries `developerInstructions` (pre-existing fix, #15); roots per process measured (premise 16). Codex's skill-root list read in `host_roots.rs` at `rust-v0.163.0`. Review pending.
- 2026-11-09 — Draft 2: user decisions U1 (a) · U2 (b) · U3 (exact wording) applied; main's rulings C1 · C2 (a) · C3 (a) · C4 applied; `toml` survey marked reported; the codex `-c` parse re-checked against `codex-rs/utils/cli/src/config_override.rs` at `rust-v0.163.0` (adds the `use_legacy_landlock` alias and the exact `trim_matches` fallback). Review pending.
- 2026-11-09 — Draft 1 written (TRD author subagent), committed as `21bc0063`. Not reviewed.
