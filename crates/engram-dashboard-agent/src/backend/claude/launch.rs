//! claude launch-material translation — turns a real [`LaunchKit`] into the [`LaunchArtifacts`] that `build_spec` loads,
//! and lays out the volatile folder for them. A Noop kit translates to nothing, so the argv stays byte-for-byte
//! today's.
//!
//! Volatile layout, written only through the confined [`LaunchFs`] (nothing in the working folder):
//! - `settings.json` = the raw claude settings merged in attach order → one `--settings`;
//! - `engram/skills/<name>` = a directory link (Windows: a junction) to each skill folder → `--plugin-dir <vol>/engram`,
//!   under which claude names the skill `engram:<name>`.
//!
//! Every file and link path is `kit.volatile_dir` joined with words, and a link path is used exactly as it was passed
//! to `link_dir` — a respelled path (case, prefix) or a path under a link is refused by the confinement, failing the
//! placement.
// ADR-0316
// ADR-0328
// ADR-0334
// ADR-0341

use std::path::{Path, PathBuf};

use engram_dashboard_platform::fs::same_path;
use engram_dashboard_platform::shell::path_is_cli_safe;
use serde_json::Value;

use super::settings::merge;
use crate::launch::{LaunchArtifacts, LaunchFs, LaunchKit, SkillRef};

const SETTINGS_FILE: &str = "settings.json";
const PLUGIN_DIR: &str = "engram";
const PLUGIN_SKILLS_DIR: &str = "skills";

/// Without it claude loads no `CLAUDE.md` or rules from an `--add-dir` folder, silently.
// ADR-0341
const ADDED_DIRS_CLAUDE_MD_ENV: (&str, &str) =
    ("CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD", "1");

/// The claude [`crate::backend::AgentBackend::materialize`].
///
/// A kit that is not real → [`LaunchArtifacts::default`]. A real kit → `real` + the instructions file as prepared +
/// the plugin folder (only with ≥ 1 skill) + the merged settings file (only with ≥ 1 raw setting) + the folder entries
/// in plan order + the folder env pair (only with ≥ 1 folder entry). A real kit with none of these writes nothing and
/// needs no volatile folder.
///
/// `Err` (nothing written) when the kit needs the volatile folder and has none, when two skills land on the same
/// link name, or when a path the argv carries (instructions file · settings file · plugin folder · folder entry) fails
/// [`path_is_cli_safe`]. `Err` after a partial write when a write fails — the settings file first, then the links; the
/// caller's release removes the volatile folder.
pub(super) fn materialize(kit: &LaunchKit, fs: &dyn LaunchFs) -> Result<LaunchArtifacts, String> {
    let plan = &kit.plan;
    if !plan.real {
        return Ok(LaunchArtifacts::default());
    }
    let volatile = || {
        kit.volatile_dir.as_deref().ok_or_else(|| {
            "real claude launch materials need a volatile folder, and the kit has none".to_string()
        })
    };
    let settings = if plan.raw_settings.is_empty() {
        None
    } else {
        let layers: Vec<&Value> = plan.raw_settings.iter().map(|raw| &raw.value).collect();
        Some((volatile()?.join(SETTINGS_FILE), merge(&layers)))
    };
    let plugin_dir = if kit.skills.is_empty() {
        None
    } else {
        Some(volatile()?.join(PLUGIN_DIR))
    };
    let links = match &plugin_dir {
        Some(dir) => skill_links(dir, &kit.skills)?,
        None => Vec::new(),
    };
    let mut argv_paths = kit
        .instructions_file
        .iter()
        .chain(settings.as_ref().map(|(path, _)| path))
        .chain(&plugin_dir)
        .chain(&plan.folders);
    if let Some(path) = argv_paths.find(|path| !path_is_cli_safe(path)) {
        return Err(format!(
            "a claude launch path holds a character cmd.exe interprets (% & ^ | < >) or is not UTF-8: {path:?}"
        ));
    }

    if let Some((path, value)) = &settings {
        fs.write_file(path, format!("{value:#}").as_bytes())
            .map_err(|e| format!("could not write the merged claude settings {path:?}: {e}"))?;
    }
    for (link, target) in &links {
        fs.link_dir(link, target)
            .map_err(|e| format!("could not link the claude skill {link:?} to {target:?}: {e}"))?;
    }

    let env = if plan.folders.is_empty() {
        Vec::new()
    } else {
        let (key, value) = ADDED_DIRS_CLAUDE_MD_ENV;
        vec![(key.to_string(), value.to_string())]
    };
    Ok(LaunchArtifacts {
        real: true,
        args: Vec::new(),
        instructions_file: kit.instructions_file.clone(),
        skill_roots: plugin_dir.into_iter().collect(),
        settings_file: settings.map(|(path, _)| path),
        add_dirs: plan.folders.clone(),
        env,
    })
}

/// `<plugin_dir>/skills/<name>` paired with the skill folder, for each skill. `<name>` = the skill name with every char
/// outside `[A-Za-z0-9_-]` replaced by `-` — claude derives a plugin skill's name from its folder name by that rule, so
/// the name is kept as claude would show it, and the name holds no separator, `:`, `.` or space. It is not checked
/// further: an empty name is refused by the dispatch's volatile-folder confinement (`Err`), and a Windows device name
/// goes to the OS as it is — measured on Windows 11 (2026-11-09, `mkdir` · `mklink /J`, not through the platform link):
/// `mkdir CON` · `COM1` · `AUX` made ordinary folders and `mklink /J LPT1` made a junction, while a `NUL` junction
/// failed. Two skills on the same link (as this OS compares paths) → `Err`; the launch check normally blocks such a
/// conflict earlier.
// ADR-0316
fn skill_links<'a>(
    plugin_dir: &Path,
    skills: &'a [SkillRef],
) -> Result<Vec<(PathBuf, &'a Path)>, String> {
    let mut links: Vec<(PathBuf, &Path)> = Vec::with_capacity(skills.len());
    for skill in skills {
        let name: String = skill
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let link = plugin_dir.join(PLUGIN_SKILLS_DIR).join(name);
        if let Some((taken, _)) = links.iter().find(|(other, _)| same_path(other, &link)) {
            return Err(format!(
                "the claude skill {:?} lands on the plugin folder another skill already took: {taken:?}",
                skill.name
            ));
        }
        links.push((link, &skill.dir));
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io;

    use engram_dashboard_platform::env::env_key_eq;
    use engram_dashboard_platform::shell::console_command;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::{ClaudeBackend, CLAUDE_PROGRAM};
    use crate::backend::AgentBackend;
    use crate::launch::{LaunchKitSource, MemLaunchFs, NoopLaunchKits, RawSetting};
    use crate::profile::{AgentCommand, AgentOutputFormat, AgentProfile, SpawnMode};
    use crate::types::{CommandSpec, ControlEndpoint, ToolGrant};

    const FOLDER_ENV: &str = ADDED_DIRS_CLAUDE_MD_ENV.0;

    /// A fixed fake root, absolute on every OS (Windows adds the current drive) and free of cmd metacharacters — the
    /// disk is never touched (`MemLaunchFs`), and a real folder such as the temp dir may hold `&` or `%` and trip the
    /// guard.
    fn root() -> PathBuf {
        std::path::absolute("/engram-fake/claude-launch").unwrap()
    }

    fn vol() -> PathBuf {
        root().join("launch").join("a-1")
    }

    fn library(name: &str) -> PathBuf {
        root().join("library").join(name)
    }

    fn command(output_format: AgentOutputFormat) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: vec!["stray".to_string()],
            output_format,
        }
    }

    fn noop_kit() -> LaunchKit {
        let profile = AgentProfile::new(
            "launch".to_string(),
            command(AgentOutputFormat::Terminal),
            root().join("work"),
            Vec::new(),
            false,
        );
        let plan = NoopLaunchKits.check(&profile).unwrap();
        NoopLaunchKits.prepare(plan, profile.id, 1).unwrap()
    }

    fn empty_real_kit() -> LaunchKit {
        let mut kit = noop_kit();
        kit.plan.real = true;
        kit.volatile_dir = Some(vol());
        kit
    }

    /// Two skills (one name holding `:`) · two raw settings (「기본」 first) · two folder entries in a non-sorted plan
    /// order.
    fn real_kit() -> LaunchKit {
        let mut kit = empty_real_kit();
        kit.instructions_file = Some(vol().join("instructions.md"));
        kit.skills = vec![
            SkillRef {
                name: "alpha".to_string(),
                dir: library("alpha"),
            },
            SkillRef {
                name: "ns:beta".to_string(),
                dir: library("beta"),
            },
        ];
        kit.plan.raw_settings = vec![
            RawSetting {
                preset: "builtin".to_string(),
                value: json!({
                    "env": {"DISABLE_AUTOUPDATER": "1"},
                    "allowedMcpServers": [{"serverName": "engram"}],
                }),
            },
            RawSetting {
                preset: "later".to_string(),
                value: json!({
                    "allowedMcpServers": [],
                    "env": {"FOO": "x"},
                    "model": "opus",
                }),
            },
        ];
        kit.plan.folders = vec![
            root().join("rules").join("zeta"),
            root().join("rules").join("alpha"),
        ];
        kit
    }

    fn materialized(kit: &LaunchKit) -> (Result<LaunchArtifacts, String>, MemLaunchFs) {
        let fs = MemLaunchFs::default();
        let result = materialize(kit, &fs);
        (result, fs)
    }

    // ── materialize ──

    /// ★Writes land only in the volatile folder★: the merged settings file and one link per skill, whose target is the
    /// skill folder as given — called on the claude backend directly, so the confinement of the dispatch does not hide a
    /// stray write. The dispatch (with the confinement) admits the same layout.
    // TRD S25 §4 (agent backend — translation)
    #[test]
    fn a_real_kit_writes_the_merged_settings_and_skill_links_inside_the_volatile_folder_only() {
        let kit = real_kit();
        let fs = MemLaunchFs::default();
        let command = command(AgentOutputFormat::Terminal);
        let artifacts = ClaudeBackend
            .materialize(&command, &kit, &root().join("work"), &fs)
            .unwrap();

        let settings = vol().join("settings.json");
        let alpha = vol().join("engram").join("skills").join("alpha");
        let beta = vol().join("engram").join("skills").join("ns-beta");
        assert_eq!(
            fs.paths(),
            vec![alpha.clone(), beta.clone(), settings.clone()]
        );
        assert!(fs.paths().iter().all(|p| p.starts_with(vol())));
        assert_eq!(fs.link_target(&alpha), Some(library("alpha")));
        assert_eq!(fs.link_target(&beta), Some(library("beta")));
        let merged = json!({
            "env": {"DISABLE_AUTOUPDATER": "1", "FOO": "x"},
            "allowedMcpServers": [{"serverName": "engram"}],
            "model": "opus",
        });
        assert_eq!(
            String::from_utf8(fs.file(&settings).unwrap()).unwrap(),
            format!("{merged:#}"),
            "pretty JSON of the attach-order merge — a later empty list keeps 「기본」's engram server"
        );

        assert_eq!(
            artifacts,
            LaunchArtifacts {
                real: true,
                args: Vec::new(),
                instructions_file: Some(vol().join("instructions.md")),
                skill_roots: vec![vol().join("engram")],
                settings_file: Some(settings),
                add_dirs: vec![
                    root().join("rules").join("zeta"),
                    root().join("rules").join("alpha")
                ],
                env: vec![(FOLDER_ENV.to_string(), "1".to_string())],
            }
        );

        let confined = MemLaunchFs::default();
        assert_eq!(
            crate::backend::materialize(&command, &kit, &root().join("work"), &confined),
            Ok(artifacts)
        );
        assert_eq!(confined.paths(), fs.paths());
    }

    /// An empty real kit carries `real` only and writes nothing — even with a volatile folder. A Noop kit carries nothing.
    #[test]
    fn an_empty_real_kit_carries_real_only_and_writes_nothing() {
        let (result, fs) = materialized(&empty_real_kit());
        assert_eq!(
            result,
            Ok(LaunchArtifacts {
                real: true,
                ..LaunchArtifacts::default()
            })
        );
        assert!(fs.paths().is_empty());

        let (result, fs) = materialized(&noop_kit());
        assert_eq!(result, Ok(LaunchArtifacts::default()));
        assert!(fs.paths().is_empty());
    }

    /// The folder env pair comes only with ≥ 1 folder entry; a real kit that writes nothing needs no volatile folder.
    // ADR-0341
    #[test]
    fn the_folder_env_pair_comes_only_with_a_folder_entry() {
        let mut kit = noop_kit();
        kit.plan.real = true;
        kit.instructions_file = Some(vol().join("instructions.md"));
        kit.plan.folders = vec![root().join("rules").join("one")];
        let (result, fs) = materialized(&kit);
        let artifacts = result.unwrap();
        assert_eq!(
            artifacts.env,
            vec![(FOLDER_ENV.to_string(), "1".to_string())]
        );
        assert_eq!(artifacts.add_dirs, vec![root().join("rules").join("one")]);
        assert!(fs.paths().is_empty());

        kit.plan.folders.clear();
        let (result, _) = materialized(&kit);
        let artifacts = result.unwrap();
        assert!(artifacts.env.is_empty());
        assert!(artifacts.add_dirs.is_empty());
    }

    #[test]
    fn a_kit_that_needs_the_volatile_folder_without_one_fails_and_writes_nothing() {
        let mut skills_only = empty_real_kit();
        skills_only.volatile_dir = None;
        skills_only.skills = real_kit().skills;
        let mut settings_only = empty_real_kit();
        settings_only.volatile_dir = None;
        settings_only.plan.raw_settings = real_kit().plan.raw_settings;
        for kit in [skills_only, settings_only] {
            let (result, fs) = materialized(&kit);
            assert!(result.is_err(), "{:?}", kit.plan.raw_settings);
            assert!(fs.paths().is_empty());
        }
    }

    /// Two skill names that become the same link name → `Err` before any write.
    // ADR-0316
    #[test]
    fn two_skills_on_one_link_name_fail_before_any_write() {
        let mut kit = real_kit();
        kit.skills = vec![
            SkillRef {
                name: "a:b".to_string(),
                dir: library("one"),
            },
            SkillRef {
                name: "a?b".to_string(),
                dir: library("two"),
            },
        ];
        let (result, fs) = materialized(&kit);
        let error = result.unwrap_err();
        assert!(error.contains("a?b"), "{error}");
        assert!(fs.paths().is_empty());

        kit.skills = vec![
            SkillRef {
                name: "Alpha".to_string(),
                dir: library("one"),
            },
            SkillRef {
                name: "alpha".to_string(),
                dir: library("two"),
            },
        ];
        let one_place = same_path(Path::new("Alpha"), Path::new("alpha"));
        let (result, fs) = materialized(&kit);
        assert_eq!(result.is_err(), one_place, "{result:?}");
        assert_eq!(fs.paths().is_empty(), one_place, "{:?}", fs.paths());
    }

    /// An empty skill name makes a link path with a trailing separator, which the confinement of the dispatch refuses.
    #[test]
    fn an_empty_skill_name_is_refused_by_the_confinement() {
        let mut kit = real_kit();
        kit.skills = vec![SkillRef {
            name: String::new(),
            dir: library("one"),
        }];
        let fs = MemLaunchFs::default();
        let result = crate::backend::materialize(
            &command(AgentOutputFormat::Terminal),
            &kit,
            &root().join("work"),
            &fs,
        );
        assert!(result.is_err(), "{result:?}");
    }

    /// ★Each path the argv carries passes the cmd guard★ — an unsafe instructions file, volatile folder (→ settings file ·
    /// plugin folder) or folder entry fails before any write.
    // ADR-0092
    // TRD S25 §1-5 (M8)
    #[test]
    fn a_cmd_unsafe_argv_path_fails_before_any_write() {
        let mut instructions = real_kit();
        instructions.instructions_file = Some(vol().join("in%structions.md"));
        let mut settings_volatile = real_kit();
        settings_volatile.skills.clear();
        settings_volatile.volatile_dir = Some(root().join("launch").join("a^1"));
        let mut plugin_volatile = real_kit();
        plugin_volatile.plan.raw_settings.clear();
        plugin_volatile.volatile_dir = Some(root().join("launch").join("a|1"));
        let mut folder = real_kit();
        folder.plan.folders.push(root().join("rules").join("R&D"));
        for kit in [instructions, settings_volatile, plugin_volatile, folder] {
            let (result, fs) = materialized(&kit);
            assert!(
                result.is_err(),
                "{:?} · {:?}",
                kit.volatile_dir,
                kit.plan.folders
            );
            assert!(fs.paths().is_empty(), "{:?}", fs.paths());
        }
    }

    /// A failed write of the merged settings fails the placement (no warn-and-continue — without it the engram MCP
    /// server permission is gone).
    // ADR-0328
    #[test]
    fn a_failed_settings_write_fails_the_placement() {
        struct RefusesFiles(MemLaunchFs);
        impl LaunchFs for RefusesFiles {
            fn write_file(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
            fn link_dir(&self, link: &Path, target: &Path) -> io::Result<()> {
                self.0.link_dir(link, target)
            }
            fn exists(&self, path: &Path) -> bool {
                self.0.exists(path)
            }
            fn is_link(&self, path: &Path) -> io::Result<bool> {
                self.0.is_link(path)
            }
        }
        let fs = RefusesFiles(MemLaunchFs::default());
        let error = materialize(&real_kit(), &fs).unwrap_err();
        assert!(error.contains("settings.json"), "{error}");
    }

    // ── argv ──

    const SID: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

    fn endpoint() -> ControlEndpoint {
        ControlEndpoint {
            agent_id: Uuid::nil(),
            epoch: 1,
            url: "http://127.0.0.1:54321/mcp".to_string(),
            token: "deadbeef".to_string(),
            config_path: Some(root().join("data").join("mcp.json")),
            send_exe: None,
            priming_file: Some(root().join("data").join("priming.md")),
            grants: vec![ToolGrant::Mcp {
                server: "engram".to_string(),
                tool: "eg_send".to_string(),
            }],
            settings_file: Some(root().join("data").join("fragment.json")),
            mail_allowed: false,
        }
    }

    fn real_spec(
        output_format: AgentOutputFormat,
        mode: SpawnMode,
        control: Option<ControlEndpoint>,
        kit: &LaunchKit,
        profile_env: Vec<(String, String)>,
    ) -> CommandSpec {
        let (artifacts, _) = materialized(kit);
        ClaudeBackend.build_spec(
            &command(output_format),
            mode,
            Some(SID),
            None,
            root().join("work"),
            profile_env,
            control,
            &artifacts.unwrap(),
        )
    }

    /// `template` split on spaces, with each `{NAME}` replaced by its path (paths may hold spaces).
    fn argv(template: &str) -> Vec<String> {
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        template
            .split(' ')
            .map(|word| match word {
                "{ZETA}" => path(root().join("rules").join("zeta")),
                "{ALPHA}" => path(root().join("rules").join("alpha")),
                "{SID}" => SID.to_string(),
                "{MCP}" => path(root().join("data").join("mcp.json")),
                "{INSTRUCTIONS}" => path(vol().join("instructions.md")),
                "{PLUGIN}" => path(vol().join("engram")),
                "{SETTINGS}" => path(vol().join("settings.json")),
                other => other.to_string(),
            })
            .collect()
    }

    /// ★Real-material argv golden★ — both modes × Fresh/Resume × control/none:
    /// - `--add-dir` pairs in plan order, first, so the always-present `--permission-mode` ends the variadic group and a
    ///   bare `extra_args` value (`stray`) never joins it;
    /// - the merged instructions at the priming spot + `--system-prompt-snapshot off`; the control channel's priming
    ///   file and settings fragment are not loaded;
    /// - `--plugin-dir` + one `--settings` at the fragment spot; `--allowedTools` last; no `--model` (the model rides in
    ///   the merged settings).
    // ADR-0316
    // ADR-0328
    // ADR-0341
    // TRD S25 §4 (agent backend — translation · folder entries)
    #[test]
    fn real_materials_produce_the_pinned_claude_argv() {
        const JSON: &str = "-p --input-format stream-json --output-format stream-json --replay-user-messages --verbose --include-partial-messages";
        let cases: Vec<(AgentOutputFormat, SpawnMode, bool, String)> = vec![
            (
                AgentOutputFormat::Terminal,
                SpawnMode::Fresh,
                true,
                "--add-dir {ZETA} --add-dir {ALPHA} --permission-mode bypassPermissions --session-id {SID} --mcp-config {MCP} --append-system-prompt-file {INSTRUCTIONS} --system-prompt-snapshot off --plugin-dir {PLUGIN} --settings {SETTINGS} stray --disallowedTools SendMessage --allowedTools mcp__engram__eg_send".to_string(),
            ),
            (
                AgentOutputFormat::Terminal,
                SpawnMode::Fresh,
                false,
                "--add-dir {ZETA} --add-dir {ALPHA} --permission-mode bypassPermissions --session-id {SID} --append-system-prompt-file {INSTRUCTIONS} --system-prompt-snapshot off --plugin-dir {PLUGIN} --settings {SETTINGS} stray".to_string(),
            ),
            (
                AgentOutputFormat::Terminal,
                SpawnMode::Resume,
                true,
                "--add-dir {ZETA} --add-dir {ALPHA} --permission-mode bypassPermissions --resume {SID} --mcp-config {MCP} --append-system-prompt-file {INSTRUCTIONS} --system-prompt-snapshot off --plugin-dir {PLUGIN} --settings {SETTINGS} stray --disallowedTools SendMessage --allowedTools mcp__engram__eg_send".to_string(),
            ),
            (
                AgentOutputFormat::Terminal,
                SpawnMode::Resume,
                false,
                "--add-dir {ZETA} --add-dir {ALPHA} --permission-mode bypassPermissions --resume {SID} --append-system-prompt-file {INSTRUCTIONS} --system-prompt-snapshot off --plugin-dir {PLUGIN} --settings {SETTINGS} stray".to_string(),
            ),
            (
                AgentOutputFormat::StreamJson,
                SpawnMode::Fresh,
                true,
                format!("--add-dir {{ZETA}} --add-dir {{ALPHA}} --permission-mode bypassPermissions {JSON} --session-id {{SID}} --mcp-config {{MCP}} --append-system-prompt-file {{INSTRUCTIONS}} --system-prompt-snapshot off --plugin-dir {{PLUGIN}} --settings {{SETTINGS}} stray --disallowedTools SendMessage --allowedTools mcp__engram__eg_send"),
            ),
            (
                AgentOutputFormat::StreamJson,
                SpawnMode::Fresh,
                false,
                format!("--add-dir {{ZETA}} --add-dir {{ALPHA}} --permission-mode bypassPermissions {JSON} --session-id {{SID}} --append-system-prompt-file {{INSTRUCTIONS}} --system-prompt-snapshot off --plugin-dir {{PLUGIN}} --settings {{SETTINGS}} stray"),
            ),
            (
                AgentOutputFormat::StreamJson,
                SpawnMode::Resume,
                true,
                format!("--add-dir {{ZETA}} --add-dir {{ALPHA}} --permission-mode bypassPermissions {JSON} --resume {{SID}} --mcp-config {{MCP}} --append-system-prompt-file {{INSTRUCTIONS}} --system-prompt-snapshot off --plugin-dir {{PLUGIN}} --settings {{SETTINGS}} stray --disallowedTools SendMessage --allowedTools mcp__engram__eg_send"),
            ),
            (
                AgentOutputFormat::StreamJson,
                SpawnMode::Resume,
                false,
                format!("--add-dir {{ZETA}} --add-dir {{ALPHA}} --permission-mode bypassPermissions {JSON} --resume {{SID}} --append-system-prompt-file {{INSTRUCTIONS}} --system-prompt-snapshot off --plugin-dir {{PLUGIN}} --settings {{SETTINGS}} stray"),
            ),
        ];
        let kit = real_kit();
        for (output_format, mode, control, template) in cases {
            let spec = real_spec(
                output_format.clone(),
                mode,
                control.then(endpoint),
                &kit,
                Vec::new(),
            );
            let (program, args) = console_command(CLAUDE_PROGRAM, argv(&template));
            let case = format!("{output_format:?} · {mode:?} · control {control}");
            assert_eq!(spec.program, program, "{case}");
            assert_eq!(spec.args, args, "{case}");
            assert!(!spec.args.iter().any(|a| a == "--model"), "{case}");
        }
    }

    /// A real kit with no materials still drops the control channel's priming file and settings fragment and still
    /// turns the snapshot off; nothing else appears.
    // ADR-0316
    // ADR-0328
    #[test]
    fn an_empty_real_kit_drops_the_priming_and_the_fragment_and_turns_the_snapshot_off() {
        let spec = real_spec(
            AgentOutputFormat::Terminal,
            SpawnMode::Fresh,
            Some(endpoint()),
            &empty_real_kit(),
            Vec::new(),
        );
        let (_, args) = console_command(
            CLAUDE_PROGRAM,
            argv("--permission-mode bypassPermissions --session-id {SID} --mcp-config {MCP} --system-prompt-snapshot off stray --disallowedTools SendMessage --allowedTools mcp__engram__eg_send"),
        );
        assert_eq!(spec.args, args);
    }

    /// ★The folder env var overrides a profile value★ (the stated exception to profile-wins) — exact key and a case
    /// variant (one key on Windows); without a folder entry the profile value stays and nothing is added.
    // ADR-0341
    // TRD S25 §1-5 (Plan and artifacts)
    #[test]
    fn the_folder_env_var_overrides_a_profile_value_only_with_a_folder_entry() {
        for profile_key in [FOLDER_ENV.to_string(), FOLDER_ENV.to_ascii_lowercase()] {
            let profile_env = vec![
                (profile_key.clone(), "0".to_string()),
                ("KEEP".to_string(), "k".to_string()),
            ];
            let spec = real_spec(
                AgentOutputFormat::Terminal,
                SpawnMode::Fresh,
                None,
                &real_kit(),
                profile_env.clone(),
            );
            let folder_vars: Vec<&(String, String)> = spec
                .env
                .iter()
                .filter(|(k, _)| env_key_eq(k, FOLDER_ENV))
                .collect();
            assert_eq!(
                folder_vars,
                vec![&(FOLDER_ENV.to_string(), "1".to_string())],
                "{profile_key}"
            );
            assert!(spec.env.contains(&("KEEP".to_string(), "k".to_string())));

            let mut no_folder = real_kit();
            no_folder.plan.folders.clear();
            let spec = real_spec(
                AgentOutputFormat::Terminal,
                SpawnMode::Fresh,
                None,
                &no_folder,
                profile_env,
            );
            assert!(spec.env.contains(&(profile_key.clone(), "0".to_string())));
            assert!(!spec
                .env
                .contains(&(FOLDER_ENV.to_string(), "1".to_string())));
            assert!(!spec.args.iter().any(|a| a == "--add-dir"));
        }
    }
}
