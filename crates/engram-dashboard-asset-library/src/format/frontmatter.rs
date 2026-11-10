//! Front matter of library `.md` files: the library's split (used for every backend's skill naming and the excerpt —
//! M13), the skill naming and collision key, and [`YamlFrontMatter`], the real parser behind agent's
//! [`FrontMatterParser`]. A backend's own front-matter rules (such as a repair pass) live in that backend's settings
//! check and reach YAML only through this parser (M63).
// ADR-0336

use engram_dashboard_agent::backend::settings_check::{
    FrontMatterError, FrontMatterParser, PathsFrontMatter, SkillFrontMatter, SubagentFrontMatter,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::tree::Node;
use super::{line_of, options};

/// A front matter found by [`split_front_matter`]: the YAML text between the fences, the 1-based line of the file its
/// first line is on, and everything after the closing fence's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontMatter<'a> {
    pub yaml: &'a str,
    pub first_line: u32,
    pub body: &'a str,
}

/// The front matter of `text`: the first line is a fence (`---` once trimmed), the YAML runs up to the next fence line.
/// `None` = no opening fence on the first line, no closing fence, or nothing between them (a block of only whitespace
/// is returned). A leading BOM is not stripped, so it hides the fence. Line ends are `\n` or `\r\n`.
///
/// The rule was copied from a backend CLI's front-matter split (TRD S25 step-7 §3-4 — history, not a backend branch).
pub fn split_front_matter(text: &str) -> Option<FrontMatter<'_>> {
    let mut lines = text.split_inclusive('\n');
    let opening = lines.next()?;
    if !is_fence(opening) {
        return None;
    }
    let yaml_start = opening.len();
    let mut offset = yaml_start;
    for line in lines {
        if is_fence(line) {
            if offset == yaml_start {
                return None;
            }
            return Some(FrontMatter {
                yaml: &text[yaml_start..offset],
                first_line: 2,
                body: &text[offset + line.len()..],
            });
        }
        offset += line.len();
    }
    None
}

/// A fence line: `---` once its line end is cut and it is trimmed (`trim` leaves U+FEFF).
fn is_fence(line: &str) -> bool {
    line.trim() == "---"
}

/// The name a library skill goes by: the front-matter `name` of its `SKILL.md` with inner whitespace runs folded to one
/// space and the ends trimmed; the folder name when there is no `SKILL.md` text, no front matter, front matter that
/// does not parse, a `name` that is a list or a map, or a name that is blank once folded (M13). A number or boolean
/// `name` is taken as its text as the file spells it (`name: 123` → `123`).
pub fn library_skill_name(skill_md: Option<&str>, folder_name: &str) -> String {
    #[derive(Deserialize)]
    struct NameOnly {
        #[serde(default)]
        name: Option<String>,
    }
    skill_md
        .and_then(split_front_matter)
        .and_then(|front| parse::<NameOnly>(front.yaml).ok())
        .and_then(|front| front.name)
        .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| folder_name.to_owned())
}

/// The collision key of a skill name: lowercased, every character outside `[a-z0-9_-]` turned into `-` (M13 · M25).
/// Two skills whose keys are equal collide.
pub fn conflict_key(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '-' => c,
            _ => '-',
        })
        .collect()
}

/// The front-matter parser agent's settings-check port is given: serde-saphyr with [`options()`]. Errors carry the
/// line within the text it was given.
#[derive(Clone, Copy, Debug, Default)]
pub struct YamlFrontMatter;

impl FrontMatterParser for YamlFrontMatter {
    fn paths(&self, text: &str) -> Result<PathsFrontMatter, FrontMatterError> {
        parse(text)
    }

    fn subagent(&self, text: &str) -> Result<SubagentFrontMatter, FrontMatterError> {
        parse(text)
    }

    fn skill(&self, text: &str) -> Result<SkillFrontMatter, FrontMatterError> {
        parse(text)
    }

    fn is_yaml_value(&self, text: &str) -> bool {
        parse::<Node>(text).is_ok()
    }
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, FrontMatterError> {
    serde_saphyr::from_str_with_options(text, options()).map_err(|error| FrontMatterError {
        message: error.without_snippet().to_string(),
        line: line_of(&error),
    })
}

#[cfg(test)]
mod tests {
    use engram_dashboard_agent::backend::settings_check::SkillFormat;
    use engram_dashboard_agent::profile::AgentOutputFormat;
    use serde_json::json;

    use super::*;
    use crate::backends::{AgentBackends, Backends};

    // ── split ──────────────────────────────────────────────────────────────────

    #[test]
    fn the_split_takes_the_lines_between_two_fences() {
        let front = split_front_matter("---\nname: a\n---\nbody\n").unwrap();
        assert_eq!(
            front,
            FrontMatter {
                yaml: "name: a\n",
                first_line: 2,
                body: "body\n"
            }
        );
        let front = split_front_matter("--- \r\nname: a\r\nx: 1\r\n  ---\r\nbody").unwrap();
        assert_eq!((front.yaml, front.body), ("name: a\r\nx: 1\r\n", "body"));
        let front = split_front_matter("---\n  \n---").unwrap();
        assert_eq!((front.yaml, front.body), ("  \n", ""));
    }

    #[test]
    fn the_split_finds_nothing_without_both_fences_or_with_an_empty_block() {
        for text in [
            "",
            "name: a\n---\n",
            "---\nname: a\n",
            "---\n---\nbody",
            "\u{feff}---\nname: a\n---\n",
            "x\n---\nname: a\n---\n",
            "----\nname: a\n----\n",
        ] {
            assert_eq!(split_front_matter(text), None, "{text:?}");
        }
    }

    // ── naming ─────────────────────────────────────────────────────────────────

    #[test]
    fn a_library_skill_is_named_by_its_front_matter_else_its_folder() {
        let cases: &[(Option<&str>, &str)] = &[
            (
                Some("---\nname:  Deploy \t  Service \n---\n"),
                "Deploy Service",
            ),
            (Some("---\nname: yes\n---\n"), "yes"),
            (Some("---\nname: 123\n---\n"), "123"),
            (
                Some("---\nname: Weird Name.v2 한글\n---\n"),
                "Weird Name.v2 한글",
            ),
            (Some("---\nname: \"  \"\n---\n"), "fold er"),
            (Some("---\nname: ~\n---\n"), "fold er"),
            (Some("---\nname: [a]\n---\n"), "fold er"),
            (Some("---\ndescription: d\n---\n"), "fold er"),
            (Some("---\nname: a\ndescription: x: y\n---\n"), "fold er"),
            (Some("\u{feff}---\nname: a\n---\n"), "fold er"),
            (Some("no front matter"), "fold er"),
            (None, "fold er"),
        ];
        for (text, expected) in cases {
            assert_eq!(&library_skill_name(*text, "fold er"), expected, "{text:?}");
        }
    }

    #[test]
    fn the_conflict_key_lowercases_and_dashes_the_rest() {
        let cases = [
            ("C-Case", "c-case"),
            ("c_case-2", "c_case-2"),
            ("Weird Name.v2 한글", "weird-name-v2---"),
            ("ns:c-colon", "ns-c-colon"),
            ("", ""),
        ];
        for (name, key) in cases {
            assert_eq!(conflict_key(name), key, "{name:?}");
        }
        assert_eq!(conflict_key("C-Case"), conflict_key("c-case"));
    }

    // ── the parser ─────────────────────────────────────────────────────────────

    #[test]
    fn paths_and_subagent_front_matter_parse_into_agent_s_types() {
        let paths = YamlFrontMatter.paths("paths:\n- src/**\n- 1\n").unwrap();
        assert_eq!(paths.paths, Some(json!(["src/**", 1])));
        assert_eq!(YamlFrontMatter.paths("paths: ~\n").unwrap().paths, None);
        assert_eq!(YamlFrontMatter.paths("other: 1\n").unwrap().paths, None);

        let agent = YamlFrontMatter
            .subagent("name: x\ndescription: [d]\nhooks:\ntools: {PreToolUse: 1}\n")
            .unwrap();
        assert_eq!(agent.name, Some(json!("x")));
        assert_eq!(agent.description, Some(json!(["d"])));
        assert!(agent.has_hooks && !agent.has_pre_tool_use && !agent.has_permission_request);
        assert_eq!(
            serde_json::Value::Object(agent.other),
            json!({"tools": {"PreToolUse": 1}})
        );
    }

    #[test]
    fn a_parse_failure_carries_the_line_within_the_given_text() {
        let error = YamlFrontMatter.paths("a: 1\nb: [\n").unwrap_err();
        assert_eq!(error.line, Some(2), "{}", error.message);
        let error = YamlFrontMatter.skill("name: a\nname: b\n").unwrap_err();
        assert_eq!(error.line, Some(2), "{}", error.message);
    }

    #[test]
    fn is_yaml_value_answers_whether_the_text_alone_parses() {
        for text in ["[a, b]", "{}", "{x: 1}", "[next, supabase]", "plain", "'q'"] {
            assert!(YamlFrontMatter.is_yaml_value(text), "{text:?}");
        }
        for text in [
            "[next,@supabase/ssr]",
            "{x",
            "@scope/pkg",
            "`cmd`",
            "[a",
            "{a: 1, a: 2}",
        ] {
            assert!(!YamlFrontMatter.is_yaml_value(text), "{text:?}");
        }
    }

    // ── codex skill golden (TRD S25 step-7 §8-1 P3 · M63 · parent §6-2 L1017) ───
    //
    // The real parser through the codex settings check — the codex word appears only here, in tests. Expected values
    // follow codex's behaviour as recorded upstream (`codex-rs/skills/src/parser_tests.rs` @ rust-v0.156.1 — its six
    // tests at L6–22, L24–43, L45–61, L63–79, L81–97 and L99–119) and in
    // `docs/research/skill-md-recognition-claude-codex-2026-10-20.md` (fixtures). Where serde-saphyr and
    // codex's serde_yaml 0.9.34 may differ, the row says so.

    fn codex_format(file: &str, folder: &str, text: &str) -> SkillFormat {
        AgentBackends
            .for_word("codex")
            .expect("codex is registered")
            .skill_format(
                file,
                folder,
                text,
                &YamlFrontMatter,
                AgentOutputFormat::StreamJson,
            )
    }

    fn skill(text: &str) -> SkillFormat {
        codex_format("SKILL.md", "fallback", text)
    }

    fn valid(name: &str, description: &str) -> SkillFormat {
        SkillFormat::Valid {
            name: name.to_owned(),
            description: description.to_owned(),
        }
    }

    fn invalid_code(format: &SkillFormat) -> Option<(&str, Option<u32>)> {
        match format {
            SkillFormat::Invalid(reason) => Some((reason.code.as_str(), reason.line)),
            _ => None,
        }
    }

    #[test]
    fn upstream_parser_test_rows() {
        // parses_repairs_and_sanitizes_frontmatter
        assert_eq!(
            skill("---\nname:  deploy  service\ndescription: Build for AWS: ECS\nmetadata:\n  short-description:  Deploy   safely\n---\n"),
            valid("deploy service", "Build for AWS: ECS")
        );
        // uses_default_name_and_requires_description
        assert_eq!(
            codex_format("SKILL.md", "demo", "---\ndescription: Demo skill\n---\n"),
            valid("demo", "Demo skill")
        );
        assert_eq!(
            invalid_code(&skill("---\nname: demo\n---\n")),
            Some(("codex-skill-description-empty", None))
        );
        // repairs_short_descriptions_containing_colons_and_apostrophes
        assert_eq!(
            skill("---\nname: short\ndescription: Short skill\nmetadata:\n  short-description: What's included: builds and tests\n---\n"),
            valid("short", "Short skill")
        );
        // repairs_unrecognized_frontmatter_fields_that_need_quotes
        assert_eq!(
            skill("---\nname: unknown\ndescription: Unknown fields\nargument-hint: <duration: e.g. 7d, 2w>\ntags: [next,@supabase/ssr]\n---\n"),
            valid("unknown", "Unknown fields")
        );
        // preserves_block_scalar_bodies_while_repairing_other_fields
        assert_eq!(
            skill("---\nname: block\ndescription: |-\n  Build for AWS: ECS\nargument-hint: <duration: e.g. 7d>\n---\n"),
            valid("block", "Build for AWS: ECS")
        );
        // preserves_overlong_descriptions_and_short_descriptions
        let description = "💡".repeat(1_025);
        let short = "x".repeat(1_025);
        assert_eq!(
            skill(&format!("---\nname: long\ndescription: {description}\nmetadata:\n  short-description: {short}\n---\n")),
            valid("long", &description)
        );
    }

    #[test]
    fn research_fixture_rows() {
        assert_eq!(
            skill("---\nname: c-exact\ndescription: d\n---\nbody"),
            valid("c-exact", "d")
        );
        for file in ["skill.md", "Skill.md"] {
            assert_eq!(
                invalid_code(&codex_format(
                    file,
                    "c-lower",
                    "---\nname: c\ndescription: d\n---\n"
                )),
                Some(("codex-skill-file-name", None)),
                "{file}"
            );
        }
        assert_eq!(
            codex_format("SKILL.md", "c-noname", "---\ndescription: d\n---\n"),
            valid("c-noname", "d")
        );
        assert_eq!(
            codex_format(
                "SKILL.md",
                "c-emptyname",
                "---\nname: \"\"\ndescription: d\n---\n"
            ),
            valid("c-emptyname", "d")
        );
        assert_eq!(
            invalid_code(&skill("---\nname: c-nodesc\n---\n")),
            Some(("codex-skill-description-empty", None))
        );
        assert_eq!(
            invalid_code(&skill("# no front matter\n")),
            Some(("codex-skill-no-front-matter", None))
        );
        // c-badyaml: broken YAML the repair cannot fix (the error line is the file's)
        assert_eq!(
            invalid_code(&skill(
                "---\nname: c-badyaml\ndescription: d\n- item\n---\n"
            )),
            Some(("codex-skill-front-matter", Some(4)))
        );
        // A value starting with `[` that is not YAML on its own is repaired into text.
        assert_eq!(
            skill("---\nname: [unclosed\ndescription: d\n---\n"),
            valid("[unclosed", "d")
        );

        assert_eq!(
            invalid_code(&skill("\u{feff}---\nname: c-bom\ndescription: d\n---\n")),
            Some(("codex-skill-no-front-matter", None))
        );
        assert_eq!(
            skill("---\r\nname: c-crlf\r\ndescription: d\r\n---\r\nbody\r\n"),
            valid("c-crlf", "d")
        );
        assert_eq!(
            codex_format(
                "SKILL.md",
                "c-dir-mismatch",
                "---\nname: c-fm-other\ndescription: d\n---\n"
            ),
            valid("c-fm-other", "d")
        );
        assert_eq!(
            skill("---\nname: Weird Name.v2 한글\ndescription: d\n---\n"),
            valid("Weird Name.v2 한글", "d")
        );
        assert_eq!(
            skill("---\nname: ns:c-colon\ndescription: d\n---\n"),
            valid("ns:c-colon", "d")
        );
        let name64 = "n".repeat(64);
        assert_eq!(
            skill(&format!("---\nname: {name64}\ndescription: d\n---\n")),
            valid(&name64, "d")
        );
        assert_eq!(
            invalid_code(&skill(&format!(
                "---\nname: {}\ndescription: d\n---\n",
                "n".repeat(65)
            ))),
            Some(("codex-skill-name-too-long", None))
        );
    }

    #[test]
    fn repair_rows_through_the_real_parser() {
        let cases: &[(&str, SkillFormat)] = &[
            // colon + space in a plain value, also after a tab
            ("description: Build: x", valid("r", "Build: x")),
            ("description:\tBuild: x", valid("r", "Build: x")),
            // values starting with `[` `{` `@` `` ` `` that are not YAML on their own, in unknown fields
            (
                "tags: [next,@supabase/ssr]\ndescription: d",
                valid("r", "d"),
            ),
            ("a: {x\ndescription: d", valid("r", "d")),
            ("a: @scope/pkg\ndescription: d", valid("r", "d")),
            ("a: `cmd`\ndescription: d", valid("r", "d")),
            // a valid flow value keeps its type and needs no repair
            (
                "tags: [a, b]\nmetadata: {}\ndescription: d",
                valid("r", "d"),
            ),
            // a trailing comment stays outside the quotes
            ("description: Build: x # note", valid("r", "Build: x")),
            // already quoted values are left alone
            ("description: 'Build: x'", valid("r", "Build: x")),
            ("description: \"Build: x\"", valid("r", "Build: x")),
            // block scalars are left alone
            (
                "description: >\n  Build: x\n  more",
                valid("r", "Build: x more"),
            ),
        ];
        for (front, expected) in cases {
            let text = format!("---\nname: r\n{front}\n---\n");
            assert_eq!(&skill(&text), expected, "{front:?}");
        }
    }

    #[test]
    fn a_wrong_typed_name_fails_after_the_repair_with_the_original_error_line() {
        // A valid flow sequence where a string belongs: the repair leaves it, codex rejects the skill.
        assert_eq!(
            invalid_code(&skill("---\nname: [foo]\ndescription: d\n---\n")),
            Some(("codex-skill-front-matter", Some(2)))
        );
        assert_eq!(
            invalid_code(&skill("---\nname: a\ndescription: d\nmetadata: 5\n---\n")),
            Some(("codex-skill-front-matter", Some(4)))
        );
    }

    #[test]
    fn yaml_edges_against_codex_s_reader() {
        // YAML 1.1 booleans are strings under `strict_booleans` (serde_yaml 0.9 reads YAML 1.2: also strings).
        for word in ["yes", "on", "off", "No"] {
            assert_eq!(
                skill(&format!("---\nname: {word}\ndescription: d\n---\n")),
                valid(word, "d"),
                "{word}"
            );
        }
        // `true` is a boolean in both readers; serde-saphyr hands its text to a string field (serde_yaml 0.9 does
        // the same for a plain scalar — not run side by side here).
        assert_eq!(
            skill("---\nname: true\ndescription: d\n---\n"),
            valid("true", "d")
        );
        // A number where a string belongs is taken as its text.
        assert_eq!(
            skill("---\nname: a\ndescription: 123\n---\n"),
            valid("a", "123")
        );
        // Non-string keys are unknown keys and ignored.
        assert_eq!(
            skill("---\n1: x\ntrue: y\nname: a\ndescription: d\n---\n"),
            valid("a", "d")
        );
        // A null name falls back to the folder name.
        assert_eq!(
            codex_format("SKILL.md", "folder", "---\nname: ~\ndescription: d\n---\n"),
            valid("folder", "d")
        );
        // Anchors and aliases expand.
        assert_eq!(
            skill("---\nname: &n a\ndescription: *n\n---\n"),
            valid("a", "a")
        );
        // A duplicate key is an error (serde_yaml: a duplicate struct field — also an error).
        assert_eq!(
            invalid_code(&skill("---\nname: a\nname: b\ndescription: d\n---\n")),
            Some(("codex-skill-front-matter", Some(3)))
        );
        // A whitespace-only block reaches the parser (codex parses it too).
        assert_eq!(
            invalid_code(&skill("---\n   \n---\n")),
            Some(("codex-skill-description-empty", None))
        );
    }
}
