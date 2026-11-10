//! The codex command-line guard and budget — what a codex launch command line can carry and whether it fits in
//! cmd's 8191 UTF-16 units. Shared by the settings-check port (`settings.rs`) and the launch translation, so the
//! check and the launch count the same line the same way.
//!
//! ★The count is an upper bound, never the real length★ — it does not copy either quoting rule (std `append_arg` in
//!   JSON mode, portable-pty `append_quoted` in terminal mode) but bounds both: per argument its UTF-16 length, plus
//!   1 for every `"` and every `\` (a `"` gets a `\` before it; backslashes before a `"` or the closing quote are
//!   doubled), plus 3 (two wrapping quotes and the separator). The priming value's backslashes are not doubled (see
//!   `needed`). Pieces only the launch knows (the program path, the resume id, the control URL) are counted at a
//!   fixed bound.
// ADR-0337

use engram_dashboard_platform::shell::text_is_cli_safe;
use serde_json::{json, Value};
use uuid::Uuid;

use super::settings::{config_lines, reason, split_config_line};
use super::{
    mcp_override_value, APPROVAL_FLAG, APPROVAL_ON_REQUEST, APP_SERVER_STDIO_FLAG,
    APP_SERVER_SUBCOMMAND, CD_FLAG, CMD_LINE_LIMIT_UTF16, CODEX_PROGRAM, CONFIG_OVERRIDE_FLAG,
    DEVELOPER_INSTRUCTIONS_KEY, MCP_URL_BUDGET, NEWLINE_REPLACEMENT, RESUME_SUBCOMMAND,
    SANDBOX_FLAG, SANDBOX_WORKSPACE_WRITE,
};
use crate::backend::settings_check::CommandLine;
use crate::launch::Reason;
use crate::profile::AgentOutputFormat;

/// The program token of the line — the `cmd.exe` path the spawn writes, quoted — counted at Windows `MAX_PATH`
/// (260) + 2 quotes.
///
/// ★Fixed on purpose, not the resolved path★: portable-pty resolves `cmd.exe` through the agent's own env `PATH`,
///   which neither the check nor the launch translation sees. A `PATH` that selects `cmd.exe` through a path longer
///   than `MAX_PATH` (an extended-length path) is not covered — no such install is known.
const PROGRAM_TOKEN_BOUND: usize = 262;

/// Room kept for the npm shim `codex.cmd`, whose line `… & "%_prog%" "%dp0%\node_modules\@openai\codex\bin\codex.js"
/// %*` cmd expands again — ★and the 8191 limit applies to that expanded line too★ (measured, M10 in
/// `docs/research/codex-launch-materials-measurements-2026-11-09.md`: a line that fits but whose expansion reaches
/// 8192 fails with "The input line is too long"). The expansion adds the node and npm folders (148 units net on the
/// measured install); a fixed margin instead of that length, because a pure check cannot read where node and npm
/// are installed.
const SHIM_MARGIN: usize = 300;

/// The switch [`engram_dashboard_platform::shell::console_command`] puts before the CLI on Windows. Counted on
/// every OS, so a line is judged the same on every machine.
const CMD_RUN_SWITCH: &str = "/c";

/// Why a priming text cannot go on the codex terminal command line — the first character the rule refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PrimingRefusal {
    pub(super) bad: char,
}

/// The arguments one raw codex value adds — `-c`, `key=value`, `-c`, `key=value`, … (flag and value alternating),
/// each line exactly as written. Each line first passes the `-c` split rule (`codex-config-line-invalid`), then the
/// guard: a line [`text_is_cli_safe`] refuses, or one with an odd number of `"`, is `codex-config-unsafe-char`.
///
/// ★Why an odd `"` count is refused★ — a `"` survives `cmd.exe /c` but flips cmd's quote state for every later
///   argument (M5); the priming argument that follows the preset lines holds `<` and `>`, safe only while that
///   state is balanced when it arrives. An even count keeps it balanced at every argument boundary: cmd toggles on
///   every `"` (`^` is refused, so no quote is caret-escaped), and both quoting rules add 0 or 2 wrapping `"` and
///   otherwise only backslashes. Any `'` is fine.
// ADR-0337
pub(super) fn config_args(raw: &Value) -> Result<Vec<String>, Reason> {
    let mut args = Vec::new();
    for (index, line) in config_lines(raw)?.into_iter().enumerate() {
        split_config_line(index, line)?;
        if !text_is_cli_safe(line) || line.matches('"').count() % 2 == 1 {
            return Err(reason(
                "codex-config-unsafe-char",
                json!({ "index": index, "line": line }),
                None,
            ));
        }
        args.push(CONFIG_OVERRIDE_FLAG.to_owned());
        args.push(line.to_owned());
    }
    Ok(args)
}

/// The codex terminal priming value `developer_instructions='…'` for `text`, or the first character that cannot
/// be carried.
///
/// ★Wrapped in a TOML literal string (single quotes)★ — it has no escapes, so the value crosses as it is, and that
///   shape is measured to survive both `cmd.exe /c` and the `.cmd` shim (ADR-0210). The price: a `'` cannot be held.
/// ★Refused rather than escaped★ — each refused character breaks a different layer: `'` the TOML literal, `"` and
///   `\` the command-line quoting around it (the spawn escapes them as `\"`, which cmd does not know — it only counts
///   quotes), `%` cmd's variable expansion (done even inside quotes). Control characters other than CR / LF are
///   refused too: cmd's tokenizing of them was never checked, and every symptom would be a silently different line.
/// ★Line breaks become the two characters `\n`★ — a real LF makes cmd cut the line there with no error, dropping
///   every later argument; the model reads the two characters as a line break (measured), and a TOML literal
///   carries them as they are.
/// ★A text with no space (U+0020) also may not hold `% & ^ | < >`★ — std and portable-pty quote an argument only when
///   it holds a space, TAB, LF, VT or `"` (or is empty), and the last four never reach the value; so a space-less
///   priming goes on the line bare and cmd acts on those characters (`a&calc&rem` runs `calc`; a sentence without
///   spaces holding `<reply>` becomes a redirection and codex never starts).
/// ★The guard reads the text before the replacement★ — after it, our own backslashes would trip it.
/// ★Do not drop the guard because it never trips today★ — the priming file is hand-edited markdown, and one English
///   contraction (`don't`) brings a `'`.
// ADR-0215
pub(super) fn priming_value(text: &str) -> Result<String, PrimingRefusal> {
    let quoted_on_the_line = text.contains(' ');
    if let Some(bad) = text.chars().find(|c| {
        let line_break = matches!(c, '\n' | '\r');
        matches!(c, '\'' | '"' | '%' | '\\')
            || (!line_break && c.is_control())
            || (!line_break && !quoted_on_the_line && !text_is_cli_safe(c.encode_utf8(&mut [0; 4])))
    }) {
        return Err(PrimingRefusal { bad });
    }
    let single_line = text
        .replace("\r\n", NEWLINE_REPLACEMENT)
        .replace('\n', NEWLINE_REPLACEMENT)
        .replace('\r', NEWLINE_REPLACEMENT);
    Ok(format!("{DEVELOPER_INSTRUCTIONS_KEY}='{single_line}'"))
}

/// Whether the whole launch command line of `line` fits in [`CMD_LINE_LIMIT_UTF16`], counted as an upper bound
/// (see the module doc). Pieces, after the wrapper (the program token at [`PROGRAM_TOKEN_BOUND`], `/c`, `codex`,
/// [`SHIM_MARGIN`]):
/// - terminal = `resume <uuid>` (always counted) + `--cd <cwd> -s workspace-write -a on-request` + `extra_args` +
///   `config_args` + the MCP attachment at its bound + the priming value of `instructions`;
/// - JSON = `app-server --stdio` + `extra_args` + `config_args` + the MCP attachment at its bound. `instructions` is
///   ignored there — neither counted nor checked — because the JSON mode sends them in the protocol, not on the line.
///
/// `Err` = `codex-priming-unsafe-char` (terminal: `instructions` holds a character [`priming_value`] refuses) or
/// `command-line-too-long` (`{needed, limit}`).
// ADR-0337
pub(super) fn fits(line: &CommandLine<'_>) -> Result<(), Reason> {
    let needed = needed(line)?;
    if needed > CMD_LINE_LIMIT_UTF16 {
        return Err(reason(
            "command-line-too-long",
            json!({ "needed": needed, "limit": CMD_LINE_LIMIT_UTF16 }),
            None,
        ));
    }
    Ok(())
}

/// One text's share of the count: its UTF-16 length with every `"` and every `\` counted twice. Also the measure of
/// the control URL's bound ([`MCP_URL_BUDGET`]).
pub(super) fn units(text: &str) -> usize {
    text.encode_utf16().count() + text.chars().filter(|c| matches!(c, '"' | '\\')).count()
}

/// The upper-bound cost of one argument: [`units`] + 2 wrapping quotes + 1 separator.
fn arg_cost(arg: &str) -> usize {
    units(arg) + 3
}

/// The wrapper's share: the program token, then `/c` and `codex`, each unquoted after a separator, and the shim.
fn wrapper_cost() -> usize {
    PROGRAM_TOKEN_BOUND + 1 + CMD_RUN_SWITCH.len() + 1 + CODEX_PROGRAM.len() + SHIM_MARGIN
}

/// The count [`fits`] compares with the limit.
fn needed(line: &CommandLine<'_>) -> Result<usize, Reason> {
    let on_the_line = match line.mode {
        AgentOutputFormat::Terminal => line.instructions,
        AgentOutputFormat::StreamJson => None,
    };
    let priming = on_the_line
        .map(priming_value)
        .transpose()
        .map_err(|refusal| {
            reason(
                "codex-priming-unsafe-char",
                json!({ "char": refusal.bad.to_string() }),
                None,
            )
        })?;
    // The priming value holds no `"` and no `\` of its own (refused above); its only backslashes are the `\n`
    // replacements, each followed by `n` — never doubled. So its exact cost is its length + 3, and a line break
    // costs 2.
    let priming_cost = priming.map_or(0, |value| {
        arg_cost(CONFIG_OVERRIDE_FLAG) + value.encode_utf16().count() + 3
    });
    Ok(wrapper_cost()
        + counted_args(line)
            .iter()
            .map(|arg| arg_cost(arg))
            .sum::<usize>()
        + priming_cost)
}

/// The arguments after `codex`, in launch order, with the bounded pieces at their bound — everything [`needed`]
/// counts except the priming value.
fn counted_args(line: &CommandLine<'_>) -> Vec<String> {
    let mut args: Vec<String> = match line.mode {
        AgentOutputFormat::Terminal => vec![
            RESUME_SUBCOMMAND.to_owned(),
            Uuid::nil().hyphenated().to_string(),
            CD_FLAG.to_owned(),
            line.cwd.to_string_lossy().into_owned(),
            SANDBOX_FLAG.to_owned(),
            SANDBOX_WORKSPACE_WRITE.to_owned(),
            APPROVAL_FLAG.to_owned(),
            APPROVAL_ON_REQUEST.to_owned(),
        ],
        AgentOutputFormat::StreamJson => vec![
            APP_SERVER_SUBCOMMAND.to_owned(),
            APP_SERVER_STDIO_FLAG.to_owned(),
        ],
    };
    args.extend(line.extra_args.iter().cloned());
    args.extend(line.config_args.iter().cloned());
    args.push(CONFIG_OVERRIDE_FLAG.to_owned());
    // A URL of `MCP_URL_BUDGET` plain characters costs exactly the bound a real URL is held to.
    args.push(mcp_override_value(&"u".repeat(MCP_URL_BUDGET)));
    args
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn line<'a>(
        mode: AgentOutputFormat,
        extra_args: &'a [String],
        config_args: &'a [String],
        instructions: Option<&'a str>,
    ) -> CommandLine<'a> {
        CommandLine {
            mode,
            cwd: Path::new("C:/w"),
            extra_args,
            config_args,
            instructions,
        }
    }

    const MODES: [AgentOutputFormat; 2] =
        [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson];

    // ── config lines ─────────────────────────────────────────────────────────────

    #[test]
    fn config_args_alternate_the_flag_and_each_line_as_written() {
        assert_eq!(
            config_args(&json!([
                "model=o3",
                " p0.sp = 5 ",
                "p0.lit='it''s'",
                r#"model="o3""#
            ])),
            Ok([
                "-c",
                "model=o3",
                "-c",
                " p0.sp = 5 ",
                "-c",
                "p0.lit='it''s'",
                "-c",
                r#"model="o3""#
            ]
            .map(str::to_owned)
            .to_vec())
        );
        assert_eq!(config_args(&json!([])), Ok(vec![]));
    }

    #[test]
    fn a_line_with_a_banned_char_or_an_odd_quote_count_is_refused() {
        let banned = ['%', '&', '^', '|', '<', '>', '\n', '\r', '\t', '\u{1}'];
        let lines: Vec<String> = banned
            .iter()
            .map(|c| format!("p0.v=a{c}b"))
            .chain([r#"model="o3"#.to_owned(), r#"p0.q="a"b""#.to_owned()])
            .collect();
        for bad in lines {
            assert_eq!(
                config_args(&json!(["ok=1", bad])),
                Err(reason(
                    "codex-config-unsafe-char",
                    json!({"index": 1, "line": bad}),
                    None
                )),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_line_codex_cannot_split_or_a_value_that_is_not_a_list_is_refused_first() {
        assert_eq!(
            config_args(&json!(["50%"])),
            Err(reason(
                "codex-config-line-invalid",
                json!({"index": 0, "line": "50%"}),
                None
            ))
        );
        assert_eq!(
            config_args(&json!({"model": "o3"})),
            Err(reason("codex-config-not-a-list", json!({}), None))
        );
    }

    /// A bare line starting with `-` is likely read by clap as an option, so codex would exit at startup (inferred: codex's `-c`
    /// does not set `allow_hyphen_values`); a line with leading whitespace would reach codex, but no codex key starts
    /// with `-`.
    #[test]
    fn a_line_that_starts_with_a_dash_is_refused() {
        for bad in [r#"-c model="o3""#, "--profile=x", "-x=1", "  -x=1"] {
            assert_eq!(
                config_args(&json!(["ok=1", bad])),
                Err(reason(
                    "codex-config-line-invalid",
                    json!({"index": 1, "line": bad}),
                    None
                )),
                "{bad:?}"
            );
        }
        assert_eq!(
            config_args(&json!(["p0.v=-1"])),
            Ok(vec!["-c".to_owned(), "p0.v=-1".to_owned()])
        );
    }

    // ── priming ──────────────────────────────────────────────────────────────────

    /// With no space the value is not quoted on the line, so cmd would act on `% & ^ | < >`.
    #[test]
    fn a_space_less_priming_may_not_hold_a_cmd_metachar() {
        for (text, bad) in [("a&calc&rem", '&'), ("返信は<reply>タグで。", '<')] {
            assert_eq!(priming_value(text), Err(PrimingRefusal { bad }), "{text}");
            assert_eq!(
                fits(&line(AgentOutputFormat::Terminal, &[], &[], Some(text))),
                Err(reason(
                    "codex-priming-unsafe-char",
                    json!({"char": bad.to_string()}),
                    None
                )),
                "{text}"
            );
        }
        for c in ['^', '|', '>'] {
            let text = format!("a{c}b");
            assert_eq!(priming_value(&text), Err(PrimingRefusal { bad: c }));
        }
        assert_eq!(
            priming_value("a&b <c>|d^e"),
            Ok("developer_instructions='a&b <c>|d^e'".to_owned())
        );
        assert_eq!(
            priming_value("line\nbreak"),
            Ok(r"developer_instructions='line\nbreak'".to_owned())
        );
    }

    /// The JSON mode sends its instructions in the protocol — a text the terminal line could not carry is neither
    /// counted nor refused there.
    #[test]
    fn a_json_line_ignores_the_instructions() {
        let text = format!("it's \"quoted\" {}", "x".repeat(CMD_LINE_LIMIT_UTF16));
        let json_line = line(AgentOutputFormat::StreamJson, &[], &[], Some(&text));
        assert_eq!(fits(&json_line), Ok(()));
        assert_eq!(
            needed(&json_line),
            needed(&line(AgentOutputFormat::StreamJson, &[], &[], None))
        );
    }

    #[test]
    fn the_priming_value_is_a_toml_literal_on_one_line() {
        assert_eq!(
            priming_value("a\r\nb\rc\nd <e>"),
            Ok(r"developer_instructions='a\nb\nc\nd <e>'".to_owned())
        );
        for bad in ['\'', '"', '%', '\\', '\t', '\u{1}', '\u{7f}'] {
            assert_eq!(
                priming_value(&format!("ok {bad} text")),
                Err(PrimingRefusal { bad }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_priming_the_rule_refuses_does_not_fit() {
        for bad in ['\'', '"', '%', '\\', '\u{1}'] {
            let text = format!("x{bad}");
            assert_eq!(
                fits(&line(AgentOutputFormat::Terminal, &[], &[], Some(&text))),
                Err(reason(
                    "codex-priming-unsafe-char",
                    json!({"char": bad.to_string()}),
                    None
                )),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_line_break_in_the_priming_counts_two_units() {
        let count =
            |text: &str| needed(&line(AgentOutputFormat::Terminal, &[], &[], Some(text))).unwrap();
        assert_eq!(count("ab\ncd") - count("abcd"), 2);
        assert_eq!(count("ab\r\ncd") - count("abcd"), 2);
        assert_eq!(
            count("abcd") - needed(&line(AgentOutputFormat::Terminal, &[], &[], None)).unwrap(),
            arg_cost("-c") + "developer_instructions='abcd'".len() + 3
        );
    }

    // ── budget ───────────────────────────────────────────────────────────────────

    #[test]
    fn the_wrapper_counts_the_program_token_bound_and_the_shim_margin() {
        assert_eq!(PROGRAM_TOKEN_BOUND, 262);
        assert_eq!(SHIM_MARGIN, 300);
        assert_eq!(wrapper_cost(), 262 + " /c".len() + " codex".len() + 300);
    }

    /// Every piece only the launch knows is counted: the resume pair on every terminal line, and the MCP attachment
    /// at its bound in both modes.
    #[test]
    fn the_resume_pair_and_the_mcp_bound_are_always_counted() {
        let mcp_bound = arg_cost("-c")
            + arg_cost(&format!(
                "mcp_servers.engram={{url='{}',bearer_token_env_var='ENGRAM_TOKEN',default_tools_approval_mode='approve'}}",
                "u".repeat(MCP_URL_BUDGET)
            ));
        let terminal: usize = [
            "resume",
            "00000000-0000-0000-0000-000000000000",
            "--cd",
            "C:/w",
            "-s",
            "workspace-write",
            "-a",
            "on-request",
        ]
        .iter()
        .map(|a| a.len() + 3)
        .sum();
        assert_eq!(
            needed(&line(AgentOutputFormat::Terminal, &[], &[], None)),
            Ok(wrapper_cost() + terminal + mcp_bound)
        );
        let json_pieces: usize = ["app-server", "--stdio"].iter().map(|a| a.len() + 3).sum();
        assert_eq!(
            needed(&line(AgentOutputFormat::StreamJson, &[], &[], None)),
            Ok(wrapper_cost() + json_pieces + mcp_bound)
        );
    }

    #[test]
    fn exactly_the_limit_fits_and_one_more_does_not_in_both_modes() {
        for mode in MODES {
            let base = needed(&line(mode, &[], &[], None)).unwrap();
            // One config value whose cost brings the line exactly to the limit.
            let filler_units = CMD_LINE_LIMIT_UTF16 - base - arg_cost("-c") - 3;
            let exact = ["-c".to_owned(), "x".repeat(filler_units)];
            assert_eq!(fits(&line(mode, &[], &exact, None)), Ok(()), "{mode:?}");
            let over = ["-c".to_owned(), "x".repeat(filler_units + 1)];
            assert_eq!(
                fits(&line(mode, &[], &over, None)),
                Err(reason(
                    "command-line-too-long",
                    json!({"needed": CMD_LINE_LIMIT_UTF16 + 1, "limit": CMD_LINE_LIMIT_UTF16}),
                    None
                )),
                "{mode:?}"
            );
            // A `"` or a `\` counts two units: the same length with one of them no longer fits.
            for doubled in ['"', '\\'] {
                let mut value = "x".repeat(filler_units - 1);
                value.push(doubled);
                let shifted = ["-c".to_owned(), value];
                assert!(
                    fits(&line(mode, &[], &shifted, None)).is_err(),
                    "{doubled:?}"
                );
            }
        }
        // The terminal priming takes its share of the same budget.
        let base = needed(&line(AgentOutputFormat::Terminal, &[], &[], Some(""))).unwrap();
        let text = "p".repeat(CMD_LINE_LIMIT_UTF16 - base);
        assert_eq!(
            fits(&line(AgentOutputFormat::Terminal, &[], &[], Some(&text))),
            Ok(())
        );
        let text = "p".repeat(CMD_LINE_LIMIT_UTF16 - base + 1);
        assert!(fits(&line(AgentOutputFormat::Terminal, &[], &[], Some(&text))).is_err());
    }

    #[test]
    fn the_cwd_and_the_pass_through_arguments_are_counted_in_the_terminal_line() {
        let base = needed(&line(AgentOutputFormat::Terminal, &[], &[], None)).unwrap();
        // 8 characters, two `"` and one `\` counted twice, 3 for the quotes and the separator.
        let extra = [r#"a "b" \c"#.to_owned()];
        assert_eq!(
            needed(&line(AgentOutputFormat::Terminal, &extra, &[], None)),
            Ok(base + 8 + 3 + 3)
        );
        let deep = CommandLine {
            cwd: Path::new(r"C:\a\b"),
            ..line(AgentOutputFormat::Terminal, &[], &[], None)
        };
        assert_eq!(needed(&deep), Ok(base - "C:/w".len() + r"C:\a\b".len() + 2));
        // JSON mode puts no working folder on its line.
        let json_line = |cwd: &'static str| CommandLine {
            cwd: Path::new(cwd),
            ..line(AgentOutputFormat::StreamJson, &[], &[], None)
        };
        assert_eq!(
            needed(&json_line("C:/w")),
            needed(&json_line(r"C:\a\b\c\d"))
        );
    }

    // ── the count is an upper bound of both quoting rules ────────────────────────

    /// Samples where the counter's bound is tightest: dense `"`, runs of `\` before a `"` and at the end of an
    /// argument, a `cwd` full of `\`, pass-through arguments with spaces and quotes.
    fn upper_bound_samples() -> Vec<(&'static str, String, Vec<String>, Vec<String>)> {
        let empties = format!("p0=[{}]", vec![r#""""#; 1700].join(","));
        vec![
            (
                "1700 empty strings",
                "C:/w".to_owned(),
                vec![],
                vec!["-c".to_owned(), empties],
            ),
            (
                "only a quote",
                "C:/w".to_owned(),
                vec!["\"".to_owned()],
                vec![],
            ),
            (
                "backslash runs",
                "C:/w".to_owned(),
                vec![
                    r#"a\\\\"b\\\\"#.to_owned(),
                    r"has space\\\\".to_owned(),
                    r#"\\\"x y"#.to_owned(),
                    r"\".to_owned(),
                ],
                vec![],
            ),
            (
                "a cwd of backslashes",
                r"C:\a\\b\c d\\\".to_owned(),
                vec![],
                vec![],
            ),
            (
                "spaces and quotes",
                "C:/w".to_owned(),
                vec![
                    r#"with space "quoted" text"#.to_owned(),
                    "x y".to_owned(),
                    String::new(),
                ],
                vec![],
            ),
        ]
    }

    /// The arguments counted for a sample and the count of their share of the line (everything but the wrapper).
    fn counted(
        mode: AgentOutputFormat,
        cwd: &str,
        extra: &[String],
        config: &[String],
    ) -> (Vec<String>, usize) {
        let line = CommandLine {
            mode,
            cwd: Path::new(cwd),
            extra_args: extra,
            config_args: config,
            instructions: None,
        };
        (counted_args(&line), needed(&line).unwrap() - wrapper_cost())
    }

    /// One argument as portable-pty writes it on a Windows command line (the MSVC argv quoting): bare unless it is
    /// empty or holds a space, TAB, LF, VT or `"`; otherwise wrapped in quotes, with a `\` before each `"` and the
    /// backslashes doubled where they come right before a `"` or the closing quote.
    // Behaviour mirrors portable-pty 0.8.1 src/cmdbuilder.rs `CommandBuilder::append_quoted` (MIT).
    fn pty_append_quoted(arg: &str, cmdline: &mut Vec<u16>) {
        if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
            cmdline.extend(arg.encode_utf16());
            return;
        }
        let backslash = u16::from(b'\\');
        let quote = u16::from(b'"');
        let mut pending_backslashes = 0;
        cmdline.push(quote);
        for unit in arg.encode_utf16() {
            if unit == backslash {
                pending_backslashes += 1;
                continue;
            }
            let written = if unit == quote {
                2 * pending_backslashes + 1
            } else {
                pending_backslashes
            };
            cmdline.extend(std::iter::repeat_n(backslash, written));
            cmdline.push(unit);
            pending_backslashes = 0;
        }
        cmdline.extend(std::iter::repeat_n(backslash, 2 * pending_backslashes));
        cmdline.push(quote);
    }

    #[test]
    fn the_count_bounds_the_portable_pty_quoting() {
        for (name, cwd, extra, config) in upper_bound_samples() {
            for mode in MODES {
                let (args, count) = counted(mode, &cwd, &extra, &config);
                let mut real = Vec::new();
                for arg in &args {
                    real.push(' ' as u16);
                    pty_append_quoted(arg, &mut real);
                }
                assert!(
                    count >= real.len(),
                    "{name} {mode:?}: counted {count} < real {}",
                    real.len()
                );
            }
        }
    }

    /// The JSON mode spawns through std `Command`; its real command line is read back by a probe child that prints
    /// the length of its own (`[Environment]::CommandLine`, i.e. `GetCommandLineW`). The arguments' share is the
    /// difference from a probe run without them.
    #[cfg(windows)]
    #[test]
    fn the_count_bounds_the_std_quoting() {
        let script =
            std::env::temp_dir().join(format!("engram-cmdline-probe-{}.ps1", Uuid::new_v4()));
        std::fs::write(
            &script,
            "[Console]::Out.Write([Environment]::CommandLine.Length)",
        )
        .expect("write the probe script");
        let probe = |args: &[String]| -> usize {
            let output = std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&script)
                .args(args)
                .output()
                .expect("run the probe");
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("probe output {output:?}"))
        };
        let baseline = probe(&[]);
        let mut failures = Vec::new();
        for (name, cwd, extra, config) in upper_bound_samples() {
            for mode in MODES {
                let (args, count) = counted(mode, &cwd, &extra, &config);
                let real = probe(&args) - baseline;
                if count < real {
                    failures.push(format!("{name} {mode:?}: counted {count} < real {real}"));
                }
            }
        }
        let _ = std::fs::remove_file(&script);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
