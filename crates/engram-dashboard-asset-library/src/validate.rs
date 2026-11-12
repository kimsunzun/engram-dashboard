//! The checks of one settings asset and one library skill before Engram hands them over: whether the command line can
//! carry a settings asset's raw field, and whether the backend loads a skill file. Both are answers of the backend's
//! [`SettingsCheck`] reached through [`Backends`]; nothing here branches on a backend word.
//!
//! ★No value check★: no key, type, range or value inside a raw field is judged — not against a schema, not by a rule
//!   of this crate. A wrong value is the CLI's to handle by its own rules, as when the CLI runs alone.
//!
//! Both checks are pure functions of the item's own content and the port; they read no disk.
// ADR-0359
// TRD S25 step-7 §3-6

use engram_dashboard_agent::backend::settings_check::SkillFormat;
use engram_dashboard_agent::backend::SettingsCheck;
use engram_dashboard_agent::launch::Reason;
use engram_dashboard_agent::profile::AgentOutputFormat;

use crate::backends::Backends;
use crate::format::{SettingsDoc, YamlFrontMatter};

/// What [`check_settings`] found in one settings asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsVerdict {
    /// Problem ②, each with its raw field's name as the file spells it: the port's refusal of that field's value, at
    /// the field's line. Empty = nothing refused.
    pub problems: Vec<(String, Reason)>,
}

/// Checks every raw field of `doc` whose name `backends` knows (ASCII case-insensitive), each on its own, with that
/// backend's [`SettingsCheck::command_line_args`]. A refusal takes the field's line — the port sees only the value, so
/// a line it gives is not one of the file. A field no backend knows is not checked (a later backend may claim it).
pub fn check_settings(doc: &SettingsDoc, backends: &dyn Backends) -> SettingsVerdict {
    let problems = doc
        .raw
        .iter()
        .filter_map(|field| {
            let reason = backends
                .for_word(&field.name)?
                .command_line_args(&field.value)
                .err()?;
            let line = Some(field.line);
            Some((field.name.clone(), Reason { line, ..reason }))
        })
        .collect();
    SettingsVerdict { problems }
}

/// The backend's format check of one library skill in `mode`, with the library's front-matter parser
/// ([`YamlFrontMatter`]). `file_name` = the file's own name, `folder` = the skill folder's name, `text` = the whole file.
pub fn check_skill(
    backend: &dyn SettingsCheck,
    mode: AgentOutputFormat,
    file_name: &str,
    folder: &str,
    text: &str,
) -> SkillFormat {
    backend.skill_format(file_name, folder, text, &YamlFrontMatter, mode)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backends::fake::{Registry, Scripted, WordBackends};
    use crate::backends::AgentBackends;
    use crate::format::parse_settings;

    /// A settings file whose first raw field is on line 3.
    fn settings(body: &str) -> SettingsDoc {
        parse_settings(&format!("kind: settings\nversion: 1\n{body}")).expect("settings parse")
    }

    /// `(field, code, line)` of every problem.
    fn problems(verdict: &SettingsVerdict) -> Vec<(&str, &str, Option<u32>)> {
        verdict
            .problems
            .iter()
            .map(|(field, reason)| (field.as_str(), reason.code.as_str(), reason.line))
            .collect()
    }

    fn codex() -> &'static dyn SettingsCheck {
        AgentBackends
            .for_word("codex")
            .expect("codex is registered")
    }

    fn claude() -> &'static dyn SettingsCheck {
        AgentBackends
            .for_word("claude")
            .expect("claude is registered")
    }

    // ── settings ────────────────────────────────────────────────────────────────

    /// The codex port's three refusals reach the verdict with the field's line and the port's own reason.
    #[test]
    fn a_codex_field_the_command_line_cannot_carry_is_a_problem_at_its_line() {
        for (body, code, args) in [
            (
                "codex:\n  model: o3\n",
                "codex-config-not-a-list",
                json!({}),
            ),
            (
                "codex: [model=o3, 7]\n",
                "codex-config-not-a-list",
                json!({ "index": 1 }),
            ),
            (
                "codex:\n  - model=o3\n  - foo\n",
                "codex-config-line-invalid",
                json!({ "index": 1, "line": "foo" }),
            ),
            (
                "codex: [\"p0.v=a&b\"]\n",
                "codex-config-unsafe-char",
                json!({ "index": 0, "line": "p0.v=a&b" }),
            ),
        ] {
            let verdict = check_settings(&settings(body), &AgentBackends);
            assert_eq!(
                verdict.problems,
                vec![(
                    "codex".to_owned(),
                    Reason {
                        code: code.to_owned(),
                        args,
                        line: Some(3),
                    }
                )],
                "{body:?}"
            );
        }
    }

    #[test]
    fn a_codex_field_the_command_line_can_carry_is_no_problem() {
        for body in ["codex: []\n", "codex: [model=o3, \"p0.q='a b'\"]\n"] {
            assert_eq!(
                check_settings(&settings(body), &AgentBackends),
                SettingsVerdict::default(),
                "{body:?}"
            );
        }
    }

    /// claude puts no settings on its command line — its raw field is never refused, whatever it holds.
    #[test]
    fn a_claude_field_of_any_value_is_no_problem() {
        for body in [
            "claude:\n  model: opus\n",
            "claude: 5\n",
            "claude: text\n",
            "claude: [1, 2]\n",
            "claude: null\n",
        ] {
            assert_eq!(
                check_settings(&settings(body), &AgentBackends),
                SettingsVerdict::default(),
                "{body:?}"
            );
        }
    }

    /// ★No value check★ — values a schema refuses (a codex number for a string key, an unknown enum word, a claude
    /// `permissions` that is a number, unknown keys at any depth) pass; the verdict is built field by field so a field
    /// added for a value finding fails to compile here.
    // ADR-0359
    #[test]
    fn a_value_a_schema_would_refuse_passes() {
        let doc = settings(concat!(
            "codex: [model=123, approval_policy=bogus, nobody.knows.this.key=1]\n",
            "claude:\n",
            "  permissions: 5\n",
            "  cleanupPeriodDays: ten\n",
            "  unknownKey:\n",
            "    deeper: [1, {x: 2}]\n",
        ));
        assert_eq!(doc.raw.len(), 2);
        assert_eq!(
            check_settings(&doc, &AgentBackends),
            SettingsVerdict { problems: vec![] }
        );
    }

    /// A field no backend knows stays in the document and reaches no port — every port method of the registry's
    /// backends panics, and only `for_word` is asked.
    #[test]
    fn a_raw_field_whose_name_no_backend_knows_is_kept_and_not_judged() {
        let doc = settings("later-backend: {unsafe: 1}\nother: [foo]\n");
        assert_eq!(
            doc.raw.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["later-backend", "other"]
        );
        assert_eq!(
            check_settings(&doc, &WordBackends::new(&["word-a", "word-b"])),
            SettingsVerdict::default()
        );
        assert_eq!(
            check_settings(&doc, &AgentBackends),
            SettingsVerdict::default()
        );
    }

    /// ★A backend agent adds later★ — a raw field named by a fake third backend goes through that backend's own
    /// `command_line_args`, matched case-insensitively, with no change here; the real backends' fields beside it are
    /// checked as before, in file order.
    #[test]
    fn a_raw_field_named_by_a_fake_third_backend_goes_through_its_port() {
        let backends = Registry::real_and(vec![Scripted::new("fakebe")]);

        let doc = settings("codex: [foo]\nFakeBE:\n  unsafe: true\nclaude: {permissions: 5}\n");
        assert_eq!(
            problems(&check_settings(&doc, &backends)),
            vec![
                ("codex", "codex-config-line-invalid", Some(3)),
                ("FakeBE", "fakebe-line-unsafe", Some(4)),
            ]
        );

        let fine = settings("fakebe: {safe: 1}\n");
        assert_eq!(check_settings(&fine, &backends), SettingsVerdict::default());
        assert!(
            check_settings(&doc, &AgentBackends)
                .problems
                .iter()
                .all(|(field, _)| field == "codex"),
            "without the fake backend its field is not judged"
        );
    }

    /// A line the port names is replaced by the field's line — the port cannot know the file.
    #[test]
    fn a_line_the_port_gives_is_replaced_by_the_field_line() {
        let backends = Registry::real_and(vec![Scripted::new("fakebe")]);
        let doc = settings("fakebe: {unsafe: 9}\n");
        assert_eq!(
            problems(&check_settings(&doc, &backends)),
            vec![("fakebe", "fakebe-line-unsafe", Some(3))]
        );
    }

    // ── skill format ────────────────────────────────────────────────────────────

    /// The port, backend and mode are passed through. The codex row's answer (an empty description) is the same under
    /// any front-matter parser; that `check_skill` passes the library parser is pinned by the P3 codex golden
    /// (`format/frontmatter.rs` `codex_format`), whose every row goes through `check_skill`.
    #[test]
    fn check_skill_runs_the_port_with_the_library_parser() {
        let empty = "---\nname: s\ndescription: \"\"\n---\nbody\n";
        let invalid = check_skill(
            codex(),
            AgentOutputFormat::StreamJson,
            "SKILL.md",
            "s",
            empty,
        );
        assert!(
            matches!(&invalid, SkillFormat::Invalid(reason) if reason.code == "codex-skill-description-empty"),
            "{invalid:?}"
        );
        assert_eq!(
            check_skill(
                claude(),
                AgentOutputFormat::StreamJson,
                "SKILL.md",
                "s",
                empty
            ),
            SkillFormat::NotChecked
        );
        assert_eq!(
            check_skill(codex(), AgentOutputFormat::Terminal, "SKILL.md", "s", empty),
            SkillFormat::NotChecked,
            "the mode reaches the port"
        );
        let fake = Scripted::new("fakebe");
        let refused = check_skill(
            &fake,
            AgentOutputFormat::Terminal,
            "SKILL.md",
            "s",
            "please refuse",
        );
        assert!(
            matches!(&refused, SkillFormat::Invalid(reason) if reason.code == "fakebe-skill-refused"),
            "{refused:?}"
        );
    }
}
