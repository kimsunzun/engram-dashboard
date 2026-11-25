//! codex launch-material translation — turns a real [`LaunchKit`] into the [`LaunchArtifacts`] that `build_spec` and
//! `open_spawn` load. A Noop kit translates to nothing, so the argv stays byte-for-byte today's.
//!
//! - Both modes: the preset `-c` lines through the hook assembly ([`cmdline::assembled`] — the same call the check
//!   makes): the raw settings minus their hooks lines, and the hook output the plan's hook components and the raw
//!   hooks lines give, with a trust entry per handler written.
//! - Terminal: the hook lines and the priming value built from the merged instructions file (which holds only the
//!   built-in priming in this mode). Nothing is written; skills, extra skill roots and folder entries are not loaded
//!   in this mode.
//! - JSON: the thread config's `hooks` object, the merged instructions text, whether it differs from what the
//!   conversation last received, and the extra skill roots — `engram/skills/<name>` in the volatile folder is a
//!   directory link (Windows: a junction) to each skill folder, passed as one root, followed by the plan's extra roots.
//!
//! ★Every refusal comes before the first write, and every refusal is `Err` (a failed spawn)★ — real materials have
//!   no warn-and-launch path: a priming the terminal line cannot carry blocks the launch. Most refusals repeat a rule
//!   the launch check (step 7) is to block earlier with `Broken`, so reaching one here means an input changed between the
//!   check and the launch. One has no check-side counterpart: a merged instructions file that cannot be read as
//!   UTF-8 (`prepare` writes it after the check, so only this read sees it). The terminal working-folder guard
//!   ([`cmdline::cwd_reaches_codex_unchanged`]) has one — the port's `working_folder_blocks` applies the same
//!   function — but no launch check calls it yet, so a folder it refuses fails every launch here.
//! Every link path is `kit.volatile_dir` joined with words and is used exactly as passed to `link_dir` — the
//! confinement refuses a respelled path or a path under a link, failing the placement.
// ADR-0334
// ADR-0337
// ADR-0341
// ADR-0351
// ADR-0353
// ADR-0420

use std::io;
use std::path::{Path, PathBuf};

use engram_dashboard_platform::fs::same_path;

use super::hook_assembly::{HookAssembly, Subject, TrustEntry};
use super::{cmdline, CODEX_SETTINGS_CHECK};
use crate::backend::settings_check::{CommandLine, ConfigOrigin, SettingsCheck};
use crate::launch::{
    HookTrust, LaunchArtifacts, LaunchFs, LaunchKit, Reason, SkillRef, SKILL_LINKS_DIR,
    SKILL_LINKS_PARENT,
};
use crate::profile::{AgentCommand, AgentOutputFormat};

/// The codex [`crate::backend::AgentBackend::materialize`]. `read` reads the merged instructions file (a seam for
/// tests — the [`LaunchFs`] port is write-only).
///
/// A kit that is not real → [`LaunchArtifacts::default`]. A real kit → `real` + from the hook assembly (both modes)
/// `config_args` (the raw lines minus their hooks lines) and `hook_trust` +, by mode:
/// - terminal: `hook_args` (the hook lines, empty when nothing is written) and `instructions_override` (only with a
///   non-blank instructions file);
/// - JSON: `hook_config` (`None` when nothing is written), `instructions_text` (only when non-blank),
///   `instructions_changed`, and `skill_roots` = the junction root (only with ≥ 1 skill, then one link per skill)
///   followed by the plan's extra skill roots.
///
/// `Err`, nothing written: a raw setting `-c` refusal (the line guard, `codex-hooks-unreadable`); an instructions
/// file that cannot be read as UTF-8 text; a hook assembly refusal (`codex-passthrough-hooks`,
/// `codex-hooks-unreadable`, `hook-path-unsafe`, terminal `codex-hook-line-refused`); terminal — a plan with
/// instruction sources, a working folder [`cmdline::cwd_reaches_codex_unchanged`] refuses (it goes on the line as
/// `--cd`), a priming the line cannot carry; JSON — a skill root that is not absolute or not UTF-8, skills without a
/// volatile folder, a skill name codex JSON refuses, two skills on one link; both — a command line over the budget
/// (with hook lines, the terminal hook budget text).
/// A raw `hooks.state` entry naming a session-flags hook is dropped by the assembly and logged here at `warn`, once
/// per launch (the check's calls do not log it).
/// `Err` after a partial write when a link fails; the caller's release removes the volatile folder.
pub(super) fn materialize(
    command: &AgentCommand,
    kit: &LaunchKit,
    cwd: &Path,
    fs: &dyn LaunchFs,
    read: &dyn Fn(&Path) -> io::Result<String>,
) -> Result<LaunchArtifacts, String> {
    if !kit.plan.real {
        return Ok(LaunchArtifacts::default());
    }
    let AgentCommand::Codex {
        extra_args,
        output_format,
    } = command
    else {
        unreachable!("CodexBackend handles only the Codex variant — a dispatch bug");
    };
    let config = preset_config_args(kit)?;
    let text = instructions_text(kit, read)?;
    match output_format {
        AgentOutputFormat::Terminal => terminal(kit, cwd, extra_args, config, text),
        AgentOutputFormat::StreamJson => json(kit, cwd, extra_args, config, text, fs),
    }
}

/// The `-c` arguments of every raw setting, in plan order, with the origin of each argument at the same index.
// ADR-0420
struct PresetConfig {
    args: Vec<String>,
    origins: Vec<ConfigOrigin>,
}

fn preset_config_args(kit: &LaunchKit) -> Result<PresetConfig, String> {
    let mut config = PresetConfig {
        args: Vec::new(),
        origins: Vec::new(),
    };
    for raw in &kit.plan.raw_settings {
        let lines = cmdline::config_args(&raw.value).map_err(|reason| {
            format!(
                "the codex raw settings of preset {:?} cannot go on the command line: {}",
                raw.preset,
                described(&reason)
            )
        })?;
        let origin = ConfigOrigin {
            source: raw.source.clone(),
            line: raw.line,
        };
        config
            .origins
            .extend(std::iter::repeat_n(origin, lines.len()));
        config.args.extend(lines);
    }
    Ok(config)
}

/// The merged instructions text — `None` without a file or when the text is blank. A read failure (including text
/// that is not UTF-8) fails the launch.
// ADR-0351
// ADR-0353
fn instructions_text(
    kit: &LaunchKit,
    read: &dyn Fn(&Path) -> io::Result<String>,
) -> Result<Option<String>, String> {
    let Some(path) = kit.instructions_file.as_deref() else {
        return Ok(None);
    };
    let text = read(path)
        .map_err(|e| format!("could not read the merged codex instructions {path:?}: {e}"))?;
    Ok(Some(text).filter(|text| !text.trim().is_empty()))
}

// ADR-0353
fn terminal(
    kit: &LaunchKit,
    cwd: &Path,
    extra_args: &[String],
    config: PresetConfig,
    text: Option<String>,
) -> Result<LaunchArtifacts, String> {
    if !kit.plan.instructions.is_empty() {
        return Err(format!(
            "a real codex terminal plan carries {} instruction source(s) — this mode loads only the built-in priming",
            kit.plan.instructions.len()
        ));
    }
    if !cmdline::cwd_reaches_codex_unchanged(cwd) {
        return Err(format!(
            "the codex working folder goes on the terminal command line (`--cd`) and cmd.exe could change it (two `%`, a `\"`, a control character, or `& ^ | < >` without a space) or it is not UTF-8: {cwd:?}"
        ));
    }
    let instructions_override = text
        .as_deref()
        .map(cmdline::priming_value)
        .transpose()
        .map_err(|refusal| {
            format!(
                "the codex priming holds a character the terminal command line cannot carry: {:?}",
                refusal.bad
            )
        })?;
    let assembly = assembled(&CommandLine {
        mode: AgentOutputFormat::Terminal,
        cwd,
        extra_args,
        config_args: &config.args,
        config_origins: &config.origins,
        hooks: &kit.plan.hooks,
        instructions: text.as_deref(),
    })?;
    Ok(LaunchArtifacts {
        real: true,
        config_args: assembly.config_args,
        hook_args: assembly.hook_args,
        hook_trust: hook_trust(assembly.expected),
        instructions_override,
        ..LaunchArtifacts::default()
    })
}

fn json(
    kit: &LaunchKit,
    cwd: &Path,
    extra_args: &[String],
    config: PresetConfig,
    text: Option<String>,
    fs: &dyn LaunchFs,
) -> Result<LaunchArtifacts, String> {
    // No text, no change to tell: a resume context needs a body.
    let instructions_changed = text.is_some()
        && kit.instructions_hash.is_some()
        && kit.instructions_hash != kit.last_applied_instructions_hash;
    let (junction_root, links) = skill_links(kit)?;
    let roots: Vec<PathBuf> = junction_root
        .into_iter()
        .chain(kit.plan.extra_skill_roots.iter().cloned())
        .collect();
    // The roots travel as JSON strings.
    if let Some(root) = roots
        .iter()
        .find(|root| !root.is_absolute() || root.to_str().is_none())
    {
        return Err(format!(
            "a codex skill root is not an absolute UTF-8 path: {root:?}"
        ));
    }
    let assembly = assembled(&CommandLine {
        mode: AgentOutputFormat::StreamJson,
        cwd,
        extra_args,
        config_args: &config.args,
        config_origins: &config.origins,
        hooks: &kit.plan.hooks,
        instructions: None,
    })?;

    for (link, target) in &links {
        fs.link_dir(link, target)
            .map_err(|e| format!("could not link the codex skill {link:?} to {target:?}: {e}"))?;
    }
    Ok(LaunchArtifacts {
        real: true,
        skill_roots: roots,
        config_args: assembly.config_args,
        hook_config: assembly.hook_config,
        hook_trust: hook_trust(assembly.expected),
        instructions_text: text,
        instructions_changed,
        ..LaunchArtifacts::default()
    })
}

/// The junction root and `<root>/<name>` paired with each skill folder — `(None, [])` without skills. codex names a
/// skill from its front matter or the link's target folder, so the link name only has to be unique; it is the skill
/// name as is, which must be one codex JSON accepts.
// ADR-0337
fn skill_links(kit: &LaunchKit) -> Result<(Option<PathBuf>, Vec<(PathBuf, &Path)>), String> {
    if kit.skills.is_empty() {
        return Ok((None, Vec::new()));
    }
    let volatile = kit.volatile_dir.as_deref().ok_or_else(|| {
        "real codex skills need a volatile folder, and the kit has none".to_string()
    })?;
    let root = volatile.join(SKILL_LINKS_PARENT).join(SKILL_LINKS_DIR);
    let mut links: Vec<(PathBuf, &Path)> = Vec::with_capacity(kit.skills.len());
    for SkillRef { name, dir } in &kit.skills {
        if !CODEX_SETTINGS_CHECK.accepts_skill_name(name, AgentOutputFormat::StreamJson) {
            return Err(format!("{name:?} is not a skill name codex JSON accepts"));
        }
        let link = root.join(name);
        if let Some((taken, _)) = links.iter().find(|(other, _)| same_path(other, &link)) {
            return Err(format!(
                "the codex skill {name:?} lands on the link another skill already took: {taken:?}"
            ));
        }
        links.push((link, dir));
    }
    Ok((Some(root), links))
}

/// [`cmdline::assembled`] as a launch error, logging the raw session-flags state entries the assembly dropped — here,
/// once per launch, rather than in the assembly the check calls up to three times.
// ADR-0337
// ADR-0420
fn assembled(line: &CommandLine<'_>) -> Result<HookAssembly, String> {
    let assembly = cmdline::assembled(line).map_err(|reason| {
        format!(
            "the codex launch command line was refused: {}",
            described(&reason)
        )
    })?;
    for key in &assembly.dropped_state_keys {
        tracing::warn!(
            "a codex settings hooks.state entry names a session-flags hook ({key:?}) — dropped: Engram writes every session-flags entry itself"
        );
    }
    Ok(assembly)
}

/// The assembly's trust entries as launch artifacts — each named by its hook component or settings asset.
// ADR-0420
fn hook_trust(expected: Vec<TrustEntry>) -> Vec<HookTrust> {
    expected
        .into_iter()
        .map(|entry| HookTrust {
            key: entry.key,
            hash: entry.hash,
            from: match entry.subject {
                Subject::Hook(path) => path,
                Subject::Source(origin) => origin.source,
                Subject::Unnamed => String::new(),
            },
        })
        .collect()
}

fn described(reason: &Reason) -> String {
    format!("{} {}", reason.code, reason.args)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::Cell;

    use engram_dashboard_platform::shell::console_command;
    use serde_json::json;
    use uuid::Uuid;

    use super::super::{CodexBackend, CODEX_PROGRAM};
    use crate::backend::hooks::HookRef;
    use crate::backend::AgentBackend;
    use crate::launch::{LaunchKitSource, MemLaunchFs, NoopLaunchKits, RawSetting, SourceRef};
    use crate::profile::{AgentProfile, SpawnMode};
    use crate::types::{CommandSpec, ControlEndpoint, ToolGrant, MCP_SERVER_NAME};

    const PRIMING: &str = "You are an Engram agent.\nReply to <mail> with eg_send.";
    const PRIMING_VALUE: &str =
        "developer_instructions='You are an Engram agent.\\nReply to <mail> with eg_send.'";

    /// A fixed fake root, absolute on every OS and free of cmd metacharacters — the disk is never touched
    /// (`MemLaunchFs` and a fake reader).
    fn root() -> PathBuf {
        std::path::absolute("/engram-fake/codex-launch").unwrap()
    }

    fn vol() -> PathBuf {
        root().join("launch").join("a-1")
    }

    fn work() -> PathBuf {
        root().join("work")
    }

    fn library(name: &str) -> PathBuf {
        root().join("library").join(name)
    }

    fn instructions_path() -> PathBuf {
        vol().join("instructions.md")
    }

    fn command(output_format: AgentOutputFormat) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: vec!["--search".to_string()],
            output_format,
        }
    }

    fn noop_kit() -> LaunchKit {
        let profile = AgentProfile::new(
            "launch".to_string(),
            command(AgentOutputFormat::Terminal),
            work(),
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

    /// Two skills · two extra skill roots · a folder entry · two raw settings (「Default」 first) · the instructions file.
    fn real_kit() -> LaunchKit {
        let mut kit = empty_real_kit();
        kit.instructions_file = Some(instructions_path());
        kit.instructions_hash = Some("k".to_string());
        kit.skills = vec![
            SkillRef {
                name: "review".to_string(),
                dir: library("review"),
            },
            SkillRef {
                name: "lint-2".to_string(),
                dir: library("lint"),
            },
        ];
        kit.plan.extra_skill_roots = vec![
            root().join("rules").join(".agents").join("skills"),
            root().join("rules").join(".codex").join("skills"),
        ];
        kit.plan.folders = vec![root().join("rules")];
        kit.plan.raw_settings = vec![
            RawSetting {
                preset: "builtin".to_string(),
                source: "builtin".to_string(),
                line: None,
                value: json!(["model_reasoning_effort=\"low\""]),
            },
            RawSetting {
                preset: "team.yaml".to_string(),
                source: "team.yaml".to_string(),
                line: None,
                value: json!(["model=\"o3\"", "features.web_search=true"]),
            },
        ];
        kit
    }

    fn preset_args() -> Vec<String> {
        [
            "-c",
            "model_reasoning_effort=\"low\"",
            "-c",
            "model=\"o3\"",
            "-c",
            "features.web_search=true",
        ]
        .map(String::from)
        .to_vec()
    }

    /// A reader that returns `text` for the instructions file and counts its calls.
    struct Reader {
        text: io::Result<String>,
        calls: Cell<usize>,
    }

    impl Reader {
        fn of(text: &str) -> Self {
            Self {
                text: Ok(text.to_string()),
                calls: Cell::new(0),
            }
        }

        fn failing(kind: io::ErrorKind) -> Self {
            Self {
                text: Err(io::Error::from(kind)),
                calls: Cell::new(0),
            }
        }

        fn read(&self, path: &Path) -> io::Result<String> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(path, instructions_path());
            match &self.text {
                Ok(text) => Ok(text.clone()),
                Err(e) => Err(io::Error::from(e.kind())),
            }
        }
    }

    fn materialized_with(
        output_format: AgentOutputFormat,
        kit: &LaunchKit,
        cwd: &Path,
        reader: &Reader,
    ) -> (Result<LaunchArtifacts, String>, MemLaunchFs) {
        let fs = MemLaunchFs::default();
        let result = materialize(&command(output_format), kit, cwd, &fs, &|p| reader.read(p));
        (result, fs)
    }

    fn materialized(
        output_format: AgentOutputFormat,
        kit: &LaunchKit,
    ) -> (Result<LaunchArtifacts, String>, MemLaunchFs) {
        materialized_with(output_format, kit, &work(), &Reader::of(PRIMING))
    }

    /// An absolute path that is not valid UTF-8.
    fn not_utf8() -> PathBuf {
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            // A lone surrogate is a valid Windows file name but not valid UTF-8.
            root().join(std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD800]))
        };
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStrExt;
            root().join(std::ffi::OsStr::from_bytes(b"a\xFF"))
        };
        assert!(path.to_str().is_none(), "the sample must not be UTF-8");
        path
    }

    const MODES: [AgentOutputFormat; 2] =
        [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson];

    // ── materialize ──

    /// A Noop kit translates to nothing in both modes — no write and no read, even with materials in it.
    #[test]
    fn a_noop_kit_translates_to_nothing() {
        let mut kit = real_kit();
        kit.plan.real = false;
        for mode in MODES {
            let reader = Reader::of(PRIMING);
            let (result, fs) = materialized_with(mode.clone(), &kit, &work(), &reader);
            assert_eq!(result, Ok(LaunchArtifacts::default()), "{mode:?}");
            assert!(fs.paths().is_empty(), "{mode:?}");
            assert_eq!(reader.calls.get(), 0, "{mode:?}");
        }
    }

    /// ★Terminal writes nothing★ — the priming value from the file and the preset lines in plan order (flags included);
    /// skills, extra roots and folder entries are ignored, and the hash plays no part.
    // ADR-0337
    #[test]
    fn a_real_terminal_kit_writes_nothing_and_carries_the_priming_and_the_preset_lines() {
        let mut kit = real_kit();
        kit.last_applied_instructions_hash = Some("h".to_string());
        let (result, fs) = materialized(AgentOutputFormat::Terminal, &kit);
        let expected = LaunchArtifacts {
            real: true,
            config_args: preset_args(),
            instructions_override: Some(PRIMING_VALUE.to_string()),
            ..LaunchArtifacts::default()
        };
        assert_eq!(result, Ok(expected.clone()));
        assert!(fs.paths().is_empty(), "{:?}", fs.paths());

        // The dispatch (confined, and with the std reader) reads the real file — absent here, so it fails closed.
        let confined = MemLaunchFs::default();
        let through_dispatch = crate::backend::materialize(
            &command(AgentOutputFormat::Terminal),
            &kit,
            &work(),
            &confined,
        );
        assert!(through_dispatch.is_err(), "{through_dispatch:?}");
        assert!(confined.paths().is_empty());

        kit.instructions_file = None;
        let (result, _) = materialized(AgentOutputFormat::Terminal, &kit);
        assert_eq!(
            result,
            Ok(LaunchArtifacts {
                instructions_override: None,
                ..expected
            })
        );
    }

    /// ★JSON links each skill under `<vol>/engram/skills/<name>` and nothing else★; the roots are the junction root then
    /// the plan's roots in order; the text and the change flag come along. The dispatch admits the same layout.
    // ADR-0337
    // ADR-0341
    #[test]
    fn a_real_json_kit_links_each_skill_inside_the_volatile_folder_only() {
        let kit = real_kit();
        let (result, fs) = materialized(AgentOutputFormat::StreamJson, &kit);
        let junction_root = vol().join("engram").join("skills");
        let review = junction_root.join("review");
        let lint = junction_root.join("lint-2");
        assert_eq!(fs.paths(), vec![lint.clone(), review.clone()]);
        assert!(fs.paths().iter().all(|p| p.starts_with(vol())));
        assert_eq!(fs.link_target(&review), Some(library("review")));
        assert_eq!(fs.link_target(&lint), Some(library("lint")));
        let artifacts = result.unwrap();
        assert_eq!(
            artifacts,
            LaunchArtifacts {
                real: true,
                skill_roots: vec![
                    junction_root,
                    root().join("rules").join(".agents").join("skills"),
                    root().join("rules").join(".codex").join("skills"),
                ],
                config_args: preset_args(),
                instructions_text: Some(PRIMING.to_string()),
                instructions_changed: true,
                ..LaunchArtifacts::default()
            }
        );

        let confined = MemLaunchFs::default();
        let placed = crate::launch::place_in_volatile(&kit, &confined, |fs| {
            materialize(
                &command(AgentOutputFormat::StreamJson),
                &kit,
                &work(),
                fs,
                &|p| Reader::of(PRIMING).read(p),
            )
        });
        assert_eq!(placed, Ok(artifacts));
        assert_eq!(confined.paths(), fs.paths());
    }

    /// `instructions_changed` = the kit hash is set and differs from the stored one.
    // ADR-0337
    #[test]
    fn instructions_changed_follows_the_hash_truth_table() {
        let cases = [
            (PRIMING, None, None, false),
            (PRIMING, Some("k"), Some("k"), false),
            (PRIMING, Some("k"), Some("h"), true),
            (PRIMING, Some("k"), None, true),
            (PRIMING, None, Some("h"), false),
            // A blank merged file is no text, so there is no change to tell even with differing hashes.
            (" \r\n\t", Some("k"), Some("h"), false),
        ];
        for (text, kit_hash, stored, changed) in cases {
            let mut kit = real_kit();
            kit.instructions_hash = kit_hash.map(String::from);
            kit.last_applied_instructions_hash = stored.map(String::from);
            let (result, _) = materialized_with(
                AgentOutputFormat::StreamJson,
                &kit,
                &work(),
                &Reader::of(text),
            );
            assert_eq!(
                result.unwrap().instructions_changed,
                changed,
                "{text:?} · {kit_hash:?} · {stored:?}"
            );
        }
    }

    /// An empty real kit carries `real` only and writes nothing, in both modes.
    #[test]
    fn an_empty_real_kit_carries_real_only_and_writes_nothing() {
        for mode in MODES {
            let (result, fs) = materialized(mode.clone(), &empty_real_kit());
            assert_eq!(
                result,
                Ok(LaunchArtifacts {
                    real: true,
                    ..LaunchArtifacts::default()
                }),
                "{mode:?}"
            );
            assert!(fs.paths().is_empty(), "{mode:?}");
        }
    }

    /// A blank instructions file is no instructions — no priming, no text.
    #[test]
    fn a_blank_instructions_file_is_none() {
        let reader = Reader::of(" \r\n\t");
        let (terminal, _) =
            materialized_with(AgentOutputFormat::Terminal, &real_kit(), &work(), &reader);
        assert_eq!(terminal.unwrap().instructions_override, None);
        let (json, _) =
            materialized_with(AgentOutputFormat::StreamJson, &real_kit(), &work(), &reader);
        assert_eq!(json.unwrap().instructions_text, None);
    }

    /// ★A read failure fails the launch before any write★ — not found, denied, and text that is not UTF-8 (what
    /// `read_to_string` answers with `InvalidData`).
    // ADR-0351
    #[test]
    fn an_unreadable_instructions_file_fails_before_any_write() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidData,
        ] {
            for mode in MODES {
                let reader = Reader::failing(kind);
                let (result, fs) = materialized_with(mode.clone(), &real_kit(), &work(), &reader);
                let error = result.unwrap_err();
                assert!(error.contains("instructions.md"), "{error}");
                assert!(fs.paths().is_empty(), "{kind:?} · {mode:?}");
            }
        }
    }

    /// A raw setting the `-c` guard refuses fails the launch before any write, naming the preset.
    #[test]
    fn a_refused_raw_setting_fails_before_any_write() {
        for value in [
            json!(["model=\"o3"]),
            json!(["a=1&calc"]),
            json!("model=o3"),
        ] {
            let mut kit = real_kit();
            kit.plan.raw_settings[1].value = value.clone();
            for mode in MODES {
                let (result, fs) = materialized(mode.clone(), &kit);
                let error = result.unwrap_err();
                assert!(error.contains("team.yaml"), "{error}");
                assert!(fs.paths().is_empty(), "{value} · {mode:?}");
            }
        }
    }

    /// A relative extra root, a root that is not UTF-8, and a junction root that is not UTF-8 (a volatile folder that is
    /// not) all fail before any write.
    // ADR-0341
    #[test]
    fn a_skill_root_that_is_not_an_absolute_utf8_path_fails_before_any_write() {
        let not_utf8 = not_utf8();

        let mut relative = real_kit();
        relative
            .plan
            .extra_skill_roots
            .push(PathBuf::from("rules").join(".agents").join("skills"));
        let mut extra_not_utf8 = real_kit();
        extra_not_utf8.plan.extra_skill_roots.push(not_utf8.clone());
        let mut junction_not_utf8 = real_kit();
        junction_not_utf8.volatile_dir = Some(not_utf8);
        for kit in [relative, extra_not_utf8, junction_not_utf8] {
            let (result, fs) = materialized(AgentOutputFormat::StreamJson, &kit);
            assert!(result.is_err(), "{:?}", kit.plan.extra_skill_roots);
            assert!(fs.paths().is_empty(), "{:?}", fs.paths());
        }
    }

    /// A skill name outside codex JSON's rule (`[a-z0-9-]`, 1–64) fails before any write; a name at 64 passes.
    // ADR-0337
    #[test]
    fn a_skill_name_codex_json_refuses_fails_before_any_write() {
        for name in ["Review", "re_view", "a:b", "", &"a".repeat(65)] {
            let mut kit = real_kit();
            kit.skills[1].name = name.to_string();
            let (result, fs) = materialized(AgentOutputFormat::StreamJson, &kit);
            assert!(result.is_err(), "{name:?}");
            assert!(fs.paths().is_empty(), "{name:?}");
        }
        let mut kit = real_kit();
        kit.skills[1].name = "a".repeat(64);
        assert!(materialized(AgentOutputFormat::StreamJson, &kit).0.is_ok());
    }

    fn notify_hook() -> HookRef {
        let serde_json::Value::Object(definition) =
            json!({"Stop": [{"hooks": [{"type": "command", "command": "node notify.js"}]}]})
        else {
            unreachable!()
        };
        HookRef {
            path: "team/hooks/notify".to_string(),
            dir: library("notify"),
            definition,
        }
    }

    const RAW_STOP: &str = "hooks.Stop=[{hooks=[{type='command',command='node raw.js'}]}]";

    /// ★The hook assembly's output reaches the artifacts★ — a hook component and a raw settings hooks line: terminal
    /// carries the hook lines (one per event, then one state line) and JSON the thread config's `hooks` object (events
    /// and `state`), never both; the raw hooks line leaves `config_args` in both modes; both modes carry the same trust
    /// entries, the raw group's first (index rule), each named by its origin. Nothing is written to disk.
    // ADR-0420
    #[test]
    fn a_hook_component_and_a_raw_hooks_line_reach_the_lines_or_the_thread_config() {
        let mut kit = real_kit();
        kit.plan.hooks = vec![notify_hook()];
        kit.plan.raw_settings[1].value =
            json!(["model=\"o3\"", RAW_STOP, "features.web_search=true"]);

        let (terminal, fs) = materialized(AgentOutputFormat::Terminal, &kit);
        let terminal = terminal.unwrap();
        assert!(fs.paths().is_empty());
        assert_eq!(terminal.config_args, preset_args());
        assert_eq!(terminal.hook_config, None);
        assert_eq!(terminal.hook_args.len(), 4, "{:?}", terminal.hook_args);
        assert_eq!(terminal.hook_args[0], "-c");
        assert!(
            terminal.hook_args[1].starts_with("hooks.Stop=["),
            "{:?}",
            terminal.hook_args
        );
        assert!(
            terminal.hook_args[1].contains("raw.js") && terminal.hook_args[1].contains("notify.js")
        );
        assert!(
            terminal.hook_args[3].starts_with("hooks.state={"),
            "{:?}",
            terminal.hook_args
        );

        let (json_mode, _) = materialized(AgentOutputFormat::StreamJson, &kit);
        let json_mode = json_mode.unwrap();
        assert_eq!(json_mode.config_args, preset_args());
        assert!(json_mode.hook_args.is_empty());
        let config = json_mode.hook_config.clone().expect("a thread config");
        assert_eq!(config["Stop"].as_array().map(Vec::len), Some(2), "{config}");
        assert_eq!(
            config["state"].as_object().map(|s| s.len()),
            Some(2),
            "{config}"
        );

        assert_eq!(terminal.hook_trust, json_mode.hook_trust);
        let from: Vec<&str> = json_mode
            .hook_trust
            .iter()
            .map(|t| t.from.as_str())
            .collect();
        assert_eq!(from, ["team.yaml", "team/hooks/notify"]);
        assert!(
            json_mode.hook_trust[0].key.ends_with(":stop:0:0"),
            "{:?}",
            json_mode.hook_trust
        );
        assert!(
            json_mode.hook_trust[1].key.ends_with(":stop:1:0"),
            "{:?}",
            json_mode.hook_trust
        );
        for entry in &json_mode.hook_trust {
            assert!(entry.hash.starts_with("sha256:"), "{entry:?}");
            assert_eq!(
                config["state"][&entry.key]["trusted_hash"],
                entry.hash.as_str()
            );
        }
    }

    /// ★What the check counts is what `materialize` puts on the line★ — fed the same inputs, the port answers the same
    /// budget refusal (`needed` included) as the launch, with hook lines counted in terminal mode, and both pass in JSON
    /// mode, where the hooks leave the line.
    // ADR-0420
    #[test]
    fn the_check_and_materialize_count_the_same_assembled_line() {
        let mut kit = real_kit();
        let handlers: Vec<serde_json::Value> = (0..40)
            .map(|n| json!({"type": "command", "command": format!("node \"D:/Library/hooks/notify/notify-{n}.js\"")}))
            .collect();
        let serde_json::Value::Object(definition) = json!({"Stop": [{"hooks": handlers}]}) else {
            unreachable!()
        };
        kit.plan.hooks = vec![HookRef {
            definition,
            ..notify_hook()
        }];
        let config = preset_config_args(&kit).unwrap();
        for mode in MODES {
            let line = CommandLine {
                mode: mode.clone(),
                cwd: &work(),
                extra_args: &["--search".to_string()],
                config_args: &config.args,
                config_origins: &config.origins,
                hooks: &kit.plan.hooks,
                instructions: match mode {
                    AgentOutputFormat::Terminal => Some(PRIMING),
                    AgentOutputFormat::StreamJson => None,
                },
            };
            let port = CODEX_SETTINGS_CHECK.command_line_fits(&line);
            let (launch, fs) = materialized(mode.clone(), &kit);
            assert!(fs.paths().is_empty() || mode == AgentOutputFormat::StreamJson);
            match mode {
                AgentOutputFormat::Terminal => {
                    let reason = port.unwrap_err();
                    assert_eq!(reason.code, "command-line-too-long");
                    assert_eq!(reason.args["hooks_counted"], true);
                    let error = launch.unwrap_err();
                    assert!(error.contains(&described(&reason)), "{error} vs {reason:?}");
                }
                AgentOutputFormat::StreamJson => {
                    assert_eq!(port, Ok(Vec::new()));
                    assert_eq!(launch.unwrap().hook_trust.len(), 40);
                }
            }
        }
    }

    /// ★U5★ — with a hook component attached, a pass-through `-c hooks…` argument fails the launch before any write in
    /// both modes; without hook inputs it passes untouched.
    // ADR-0420
    #[test]
    fn a_passthrough_hooks_argument_with_hook_inputs_fails_before_any_write() {
        let passthrough = AgentCommand::Codex {
            extra_args: vec!["-c".to_string(), "hooks.Stop=[]".to_string()],
            output_format: AgentOutputFormat::Terminal,
        };
        for output_format in MODES {
            let command = match &passthrough {
                AgentCommand::Codex { extra_args, .. } => AgentCommand::Codex {
                    extra_args: extra_args.clone(),
                    output_format: output_format.clone(),
                },
                _ => unreachable!(),
            };
            let mut kit = real_kit();
            kit.plan.hooks = vec![notify_hook()];
            let fs = MemLaunchFs::default();
            let error = materialize(&command, &kit, &work(), &fs, &|p| {
                Reader::of(PRIMING).read(p)
            })
            .unwrap_err();
            assert!(
                error.contains("codex-passthrough-hooks"),
                "{output_format:?}: {error}"
            );
            assert!(fs.paths().is_empty(), "{output_format:?}");

            kit.plan.hooks.clear();
            let fs = MemLaunchFs::default();
            let artifacts = materialize(&command, &kit, &work(), &fs, &|p| {
                Reader::of(PRIMING).read(p)
            })
            .unwrap();
            assert!(artifacts.hook_args.is_empty() && artifacts.hook_config.is_none());
        }
    }

    /// ★A dropped raw session-flags state entry is logged once per launch★ — by `materialize`, at `warn`; the check's
    /// assembly (called up to three times by the launch check) logs nothing.
    // ADR-0420
    #[test]
    fn a_dropped_session_flags_state_entry_is_logged_once_by_materialize_only() {
        use crate::log_capture::Capture;
        use std::sync::{Arc, Mutex};

        let key = if engram_dashboard_platform::os::windows_paths() {
            r"C:\<session-flags>\config.toml:stop:0:0"
        } else {
            "/<session-flags>/config.toml:stop:0:0"
        };
        let mut kit = real_kit();
        kit.plan.raw_settings[1].value =
            json!([format!("hooks.state={{'{key}'={{enabled=true}}}}")]);
        let config = preset_config_args(&kit).unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(Capture(Arc::clone(&lines)), || {
            let line = CommandLine {
                mode: AgentOutputFormat::StreamJson,
                cwd: &work(),
                extra_args: &[],
                config_args: &config.args,
                config_origins: &config.origins,
                hooks: &[],
                instructions: None,
            };
            for _ in 0..3 {
                assert_eq!(
                    CODEX_SETTINGS_CHECK.command_line_fits(&line),
                    Ok(Vec::new())
                );
            }
            assert!(
                lines.lock().unwrap().is_empty(),
                "{:?}",
                lines.lock().unwrap()
            );
            let (artifacts, _) = materialized(AgentOutputFormat::StreamJson, &kit);
            assert_eq!(artifacts.unwrap().hook_config, None);
        });
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].0, tracing::Level::WARN);
        assert!(lines[0].1["message"].contains("session-flags"), "{lines:?}");
    }

    /// Two skills on one link fail before any write.
    #[test]
    fn two_skills_on_one_link_fail_before_any_write() {
        let mut kit = real_kit();
        kit.skills[1].name = "review".to_string();
        let (result, fs) = materialized(AgentOutputFormat::StreamJson, &kit);
        let error = result.unwrap_err();
        assert!(error.contains("review"), "{error}");
        assert!(fs.paths().is_empty());
    }

    /// Skills without a volatile folder fail before any write.
    #[test]
    fn skills_without_a_volatile_folder_fail() {
        let mut kit = real_kit();
        kit.volatile_dir = None;
        let (result, fs) = materialized(AgentOutputFormat::StreamJson, &kit);
        assert!(result.is_err());
        assert!(fs.paths().is_empty());
    }

    /// A failed link fails the placement.
    // ADR-0351
    #[test]
    fn a_failed_link_fails_the_placement() {
        struct RefusesLinks(MemLaunchFs);
        impl LaunchFs for RefusesLinks {
            fn write_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
                self.0.write_file(path, bytes)
            }
            fn link_dir(&self, _link: &Path, _target: &Path) -> io::Result<()> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
            fn exists(&self, path: &Path) -> bool {
                self.0.exists(path)
            }
            fn is_link(&self, path: &Path) -> io::Result<bool> {
                self.0.is_link(path)
            }
        }
        let fs = RefusesLinks(MemLaunchFs::default());
        let error = materialize(
            &command(AgentOutputFormat::StreamJson),
            &real_kit(),
            &work(),
            &fs,
            &|p| Reader::of(PRIMING).read(p),
        )
        .unwrap_err();
        assert!(error.contains("review"), "{error}");
    }

    /// A command line over the budget fails before any write, in both modes.
    // ADR-0337
    #[test]
    fn a_line_over_the_budget_fails_before_any_write() {
        let mut kit = real_kit();
        kit.plan.raw_settings[1].value = json!([format!("a={}", "x".repeat(8000))]);
        for mode in MODES {
            let (result, fs) = materialized(mode.clone(), &kit);
            let error = result.unwrap_err();
            assert!(error.contains("command-line-too-long"), "{error}");
            assert!(fs.paths().is_empty(), "{mode:?}");
        }
    }

    /// A real terminal plan with instruction sources is refused — the mode loads only the built-in priming.
    #[test]
    fn a_real_terminal_plan_with_instruction_sources_fails() {
        let mut kit = real_kit();
        kit.plan.instructions = vec![SourceRef {
            path: "team/rules.md".to_string(),
            file: library("rules.md"),
        }];
        let (result, fs) = materialized(AgentOutputFormat::Terminal, &kit);
        assert!(result.is_err());
        assert!(fs.paths().is_empty());
    }

    /// ★No warn-and-launch on real materials★ — a priming the rule refuses, and a line over the budget only because of
    /// the priming, both fail the launch (the Noop path carries no priming at all — ADR-0370).
    // ADR-0337
    #[test]
    fn a_priming_the_terminal_line_cannot_carry_fails_the_launch() {
        for text in [
            "don't",
            "a \"quoted\" word",
            "100% sure",
            "C:\\path",
            "bell\u{7}",
        ] {
            let (result, _) = materialized_with(
                AgentOutputFormat::Terminal,
                &real_kit(),
                &work(),
                &Reader::of(text),
            );
            assert!(result.is_err(), "{text:?}");
        }

        let mut kit = real_kit();
        kit.plan.raw_settings[1].value = json!([format!("a={}", "x".repeat(6000))]);
        let long_priming = "word ".repeat(400);
        assert!(
            materialized_with(
                AgentOutputFormat::Terminal,
                &kit,
                &work(),
                &Reader::of("short")
            )
            .0
            .is_ok(),
            "the line fits without the long priming"
        );
        let (result, _) = materialized_with(
            AgentOutputFormat::Terminal,
            &kit,
            &work(),
            &Reader::of(&long_priming),
        );
        let error = result.unwrap_err();
        assert!(error.contains("command-line-too-long"), "{error}");
    }

    /// ★A working folder cmd would change fails the real terminal launch★ — two `%`, a `"`, a control character or a
    /// non-UTF-8 path anywhere, and `& ^ | < >` only without a space (with one the `--cd` value is quoted); a single
    /// `%` passes. JSON does not put the folder on the line, and the Noop path is unchanged.
    // ADR-0337
    #[test]
    fn a_working_folder_cmd_would_change_fails_the_real_terminal_launch_only() {
        let reader = Reader::of(PRIMING);
        let refused = [
            PathBuf::from(r"C:\Work\R&D"),
            PathBuf::from(r"C:\Work\a^b"),
            PathBuf::from(r"C:\Work\a|b"),
            PathBuf::from(r"C:\Work\a<b>"),
            PathBuf::from(r"D:\%USERNAME% Projects"),
            PathBuf::from(r"C:\%A%\b"),
            PathBuf::from(r#"D:\a b"&calc"#),
            PathBuf::from("D:\\a b\u{7}"),
            not_utf8(),
        ];
        for cwd in &refused {
            let (result, fs) =
                materialized_with(AgentOutputFormat::Terminal, &real_kit(), cwd, &reader);
            let error = result.unwrap_err();
            assert!(error.contains("--cd"), "{cwd:?}: {error}");
            assert!(fs.paths().is_empty(), "{cwd:?}");

            let (result, _) =
                materialized_with(AgentOutputFormat::StreamJson, &real_kit(), cwd, &reader);
            assert!(result.is_ok(), "{cwd:?}: {result:?}");

            let mut noop = real_kit();
            noop.plan.real = false;
            let (result, _) = materialized_with(AgentOutputFormat::Terminal, &noop, cwd, &reader);
            assert_eq!(result, Ok(LaunchArtifacts::default()), "{cwd:?}");
        }

        for cwd in [
            r"D:\R&D Projects",
            r"D:\a ^ | < > b",
            r"C:\Work\100%",
            r"D:\100% Projects",
        ] {
            let (result, _) = materialized_with(
                AgentOutputFormat::Terminal,
                &real_kit(),
                Path::new(cwd),
                &reader,
            );
            assert!(result.is_ok(), "{cwd:?}: {result:?}");
        }
    }

    /// ★The port's working-folder check and `materialize` agree★ — fed the same cwd, the port blocks a mode exactly
    /// when the real launch of that mode fails on the working folder, on every folder of the table.
    // ADR-0353
    #[test]
    fn the_port_working_folder_check_and_materialize_agree() {
        let reader = Reader::of(PRIMING);
        let folders = [
            PathBuf::from(r"C:\Work\R&D"),
            PathBuf::from(r"C:\Work\a^b"),
            PathBuf::from(r"C:\Work\a|b"),
            PathBuf::from(r"C:\Work\a<b>"),
            PathBuf::from(r"D:\%USERNAME% Projects"),
            PathBuf::from(r"C:\%A%\b"),
            PathBuf::from(r#"D:\a b"&calc"#),
            PathBuf::from("D:\\a b\u{7}"),
            PathBuf::from("D:\\a\nb"),
            not_utf8(),
            PathBuf::from(r"D:\R&D Projects"),
            PathBuf::from(r"D:\a ^ | < > b"),
            PathBuf::from(r"C:\Work\100%"),
            PathBuf::from(r"D:\100% Projects"),
            PathBuf::from(r"D:\%"),
            work(),
        ];
        let mut blocked = 0;
        for cwd in &folders {
            for mode in MODES {
                let port = CODEX_SETTINGS_CHECK.working_folder_blocks(cwd, mode);
                let (launch, _) = materialized_with(mode, &real_kit(), cwd, &reader);
                assert_eq!(
                    port.is_some(),
                    launch.is_err(),
                    "{cwd:?} {mode:?}: {launch:?}"
                );
                if let (Some(reason), Err(error)) = (&port, &launch) {
                    assert_eq!(reason.code, "codex-working-folder-unsafe", "{cwd:?}");
                    assert!(error.contains("--cd"), "{cwd:?}: {error}");
                    blocked += 1;
                }
            }
        }
        assert_eq!(
            blocked, 10,
            "every refused folder of the table blocks the terminal mode"
        );
    }

    // ── argv ──

    const SID: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);

    fn endpoint() -> ControlEndpoint {
        ControlEndpoint {
            agent_id: Uuid::nil(),
            epoch: 1,
            url: "http://127.0.0.1:7777/mcp".to_string(),
            token: "deadbeef".to_string(),
            config_path: None,
            send_exe: None,
            grants: vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: "eg_send".to_string(),
            }],
            mail_allowed: false,
        }
    }

    fn mcp_value() -> String {
        super::super::mcp_override_value("http://127.0.0.1:7777/mcp")
    }

    fn real_spec(
        output_format: AgentOutputFormat,
        mode: SpawnMode,
        control: Option<ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> CommandSpec {
        CodexBackend.build_spec(
            &command(output_format),
            mode,
            None,
            Some(SID),
            work(),
            Vec::new(),
            control,
            artifacts,
        )
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    /// ★Real argv order★ — base args, pass-through, preset `-c` lines, MCP, priming (terminal); never `-m`.
    // ADR-0337
    #[test]
    fn real_materials_produce_the_pinned_codex_argv() {
        let control = endpoint();

        let (terminal, _) = materialized(AgentOutputFormat::Terminal, &real_kit());
        let terminal = terminal.unwrap();
        let (json, _) = materialized(AgentOutputFormat::StreamJson, &real_kit());
        let json = json.unwrap();
        let work = work().to_string_lossy().into_owned();
        let base = |resume: bool| {
            let mut args = Vec::new();
            if resume {
                args.extend(strings(&["resume", &SID.to_string()]));
            }
            args.extend(strings(&[
                "--cd",
                &work,
                "-s",
                "workspace-write",
                "-a",
                "on-request",
                "--search",
            ]));
            args
        };
        let cases = [
            (AgentOutputFormat::Terminal, SpawnMode::Fresh, &terminal, {
                let mut args = base(false);
                args.extend(preset_args());
                args.extend(["-c".to_string(), mcp_value()]);
                args.extend(["-c".to_string(), PRIMING_VALUE.to_string()]);
                args
            }),
            (AgentOutputFormat::Terminal, SpawnMode::Resume, &terminal, {
                let mut args = base(true);
                args.extend(preset_args());
                args.extend(["-c".to_string(), mcp_value()]);
                args.extend(["-c".to_string(), PRIMING_VALUE.to_string()]);
                args
            }),
            (AgentOutputFormat::StreamJson, SpawnMode::Fresh, &json, {
                let mut args = strings(&["app-server", "--stdio", "--search"]);
                args.extend(preset_args());
                args.extend(["-c".to_string(), mcp_value()]);
                args
            }),
            (AgentOutputFormat::StreamJson, SpawnMode::Resume, &json, {
                let mut args = strings(&["app-server", "--stdio", "--search"]);
                args.extend(preset_args());
                args.extend(["-c".to_string(), mcp_value()]);
                args
            }),
        ];
        for (output_format, mode, artifacts, expected) in cases {
            let spec = real_spec(
                output_format.clone(),
                mode,
                Some(control.clone()),
                artifacts,
            );
            let (program, args) = console_command(CODEX_PROGRAM, expected);
            let case = format!("{output_format:?} · {mode:?}");
            assert_eq!(spec.program, program, "{case}");
            assert_eq!(spec.args, args, "{case}");
            assert!(!spec.args.iter().any(|a| a == "-m"), "{case}");
        }

        // A real terminal kit without instructions carries no priming at all.
        let bare = LaunchArtifacts {
            instructions_override: None,
            ..terminal
        };
        let spec = real_spec(
            AgentOutputFormat::Terminal,
            SpawnMode::Fresh,
            Some(control),
            &bare,
        );
        assert!(
            !spec
                .args
                .iter()
                .any(|a| a.starts_with("developer_instructions=")),
            "{:?}",
            spec.args
        );
    }

    /// A preset line that sets `developer_instructions` or the engram MCP server stays on the line, and ours come later
    /// and win (the last `-c` wins).
    // ADR-0337
    #[test]
    fn our_overrides_follow_a_preset_line_for_the_same_key() {
        let mut kit = real_kit();
        kit.plan.raw_settings = vec![RawSetting {
            preset: "team.yaml".to_string(),
            source: "team.yaml".to_string(),
            line: None,
            value: json!([
                "developer_instructions='theirs'",
                "mcp_servers.engram.url='http://example.invalid/mcp'",
            ]),
        }];
        let (artifacts, _) = materialized(AgentOutputFormat::Terminal, &kit);
        let spec = real_spec(
            AgentOutputFormat::Terminal,
            SpawnMode::Fresh,
            Some(endpoint()),
            &artifacts.unwrap(),
        );
        let theirs = spec
            .args
            .iter()
            .position(|a| a == "developer_instructions='theirs'")
            .expect("the preset line stays");
        let their_mcp = spec
            .args
            .iter()
            .position(|a| a.starts_with("mcp_servers.engram.url="))
            .expect("the preset line stays");
        let our_mcp = spec.args.iter().position(|a| *a == mcp_value()).unwrap();
        let ours = spec.args.iter().position(|a| a == PRIMING_VALUE).unwrap();
        assert!(theirs < ours && their_mcp < our_mcp, "{:?}", spec.args);
        assert_eq!(spec.args.last().map(String::as_str), Some(PRIMING_VALUE));
    }
}
