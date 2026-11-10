//! OS rules for launching console programs — the default interactive shell · wrapping a CLI in that OS's console shell ·
//! which path and text values survive that wrapping. Uses only std.

use std::path::Path;

#[cfg(windows)]
pub fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
pub fn default_shell() -> &'static str {
    "bash"
}

/// Turns one CLI program into the `(program, args)` to launch — Windows = `("cmd.exe", ["/c", program, args…])`,
/// elsewhere = as received.
///
/// **Why wrap:** a CLI that npm installs on Windows is an extensionless shim (+ `.cmd`), so `CreateProcessW` (ConPTY also
/// uses it) cannot launch it directly (error 193 — it does neither PATHEXT nor shell interpretation). Wrapping it as `cmd.exe /c <prog> …`
/// lets cmd resolve the `<prog>.cmd` shim and launch the real process. `cmd /c` ends together with its target, so
/// the 「띄운 자식 = 그 CLI」 (launched child = that CLI) lifetime is kept (killing the whole tree is done by the Job Object).
///
/// - **CLI (shim) only** — do not use it for an executable that can be launched directly (`cmd.exe` itself and the like).
/// - **Known limitation — arguments are not escaped by cmd rules.** On Windows the arguments are carried as is on the
///   command line of `cmd.exe`, and an argument containing cmd metacharacters (`%NAME%` — expanded as an environment variable even inside quotes · `& ^ | < >`)
///   or the `"` and `\` that cmd counts as quotes may reach it changed, as cmd interprets them. The decision not to fix it and its
///   rationale are authoritatively in the comment 「알려진 한계 — `%VAR%` 가 든 경로는 shim 을
///   지나며 치환된다」 inside `build_spec` in agent `backend/codex/mod.rs`. Mitigation is placed by the caller per value — e.g.
///   [`path_is_cli_safe`] · [`text_is_cli_safe`] · codex's `developer_instructions_override` · `mcp_attachment`
///   rejection · passing claude `--settings` as a file path instead of inline JSON.
/// - A process launched with this command is, on Windows, the `cmd.exe` wrapper rather than that CLI — the CLI itself is a descendant under it.
// ADR-0230
pub fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let mut wrapped = Vec::with_capacity(args.len() + 2);
        wrapped.push("/c".to_string());
        wrapped.push(program.to_string());
        wrapped.extend(args);
        ("cmd.exe".to_string(), wrapped)
    }
    #[cfg(not(windows))]
    {
        (program.to_string(), args)
    }
}

/// Characters that `cmd.exe /c` may interpret in an argument (see [`console_command`]'s known limitation): `%NAME%`
/// is expanded as an environment variable even inside quotes, and `& ^ | < >` are shell metacharacters outside
/// quotes — an argument without whitespace is passed unquoted, so a path holding one of them reaches cmd bare.
// ADR-0092
const CMD_UNSAFE_CHARS: &[char] = &['%', '&', '^', '|', '<', '>'];

/// Whether a path reaches a CLI wrapped by [`console_command`] unchanged — `false` when it holds one of `% & ^ | < >`
/// or is not valid UTF-8 (arguments are carried as text, so a lossy conversion would hand the CLI a
/// different path).
///
/// The rule is the same on every OS (no OS branch) — a path that passes here passes on Windows, where the wrapping
/// happens. A caller that gets `false` must not pass the path at all; it is not escaped here.
// ADR-0092
pub fn path_is_cli_safe(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    !text.chars().any(|c| CMD_UNSAFE_CHARS.contains(&c))
}

/// Whether a text argument reaches a CLI wrapped by [`console_command`] unchanged — `false` when it holds one of
/// `% & ^ | < >` or a control character (an LF silently truncates the rest of the command line).
///
/// `"` and `'` pass: a `"` survives the wrapping but flips cmd's quote state for every later argument, so a caller
/// that places several texts on one line needs its own rule for `"` as well (e.g. an even count per argument).
/// The rule is the same on every OS (no OS branch). A caller that gets `false` must not pass the text at all.
// ADR-0092
pub fn text_is_cli_safe(text: &str) -> bool {
    !text
        .chars()
        .any(|c| CMD_UNSAFE_CHARS.contains(&c) || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_cli_safe_rejects_each_metachar() {
        for c in CMD_UNSAFE_CHARS {
            let path = std::path::PathBuf::from(format!("C:/base/na{c}me/agent.md"));
            assert!(!path_is_cli_safe(&path), "{c:?}");
        }
        assert!(path_is_cli_safe(Path::new(
            "C:/base/prompts/agent-priming.md"
        )));
        assert!(path_is_cli_safe(Path::new(
            r"C:\Users\a b\AppData\Local\engram\vol (1)\settings.json"
        )));
    }

    #[test]
    fn path_is_cli_safe_rejects_a_path_that_is_not_utf8() {
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            // A lone surrogate is a valid Windows file name but not valid UTF-8.
            std::path::PathBuf::from(std::ffi::OsString::from_wide(&[
                u16::from(b'C'),
                u16::from(b':'),
                u16::from(b'/'),
                0xD800,
            ]))
        };
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStrExt;
            std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/base/\xFF"))
        };
        assert!(path.to_str().is_none(), "the sample must not be UTF-8");
        assert!(!path_is_cli_safe(&path));
    }

    #[test]
    fn text_is_cli_safe_rejects_each_metachar_and_every_control_char() {
        let metachars = ['%', '&', '^', '|', '<', '>'];
        let controls = ['\n', '\r', '\t', '\u{0}', '\u{1b}', '\u{7f}', '\u{85}'];
        for c in metachars.into_iter().chain(controls) {
            assert!(!text_is_cli_safe(&format!("model=o{c}3")), "{c:?}");
        }
        for text in [
            r#"model="o3""#,
            "p0.lit='it''s'",
            "a b",
            "",
            "!bang",
            r"\\server\x",
            "한글",
        ] {
            assert!(text_is_cli_safe(text), "{text:?}");
        }
    }

    /// ★Arguments are carried in order without changing a single character★ — arguments with spaces or cmd metacharacters also get no quotes or escapes
    /// added (it measures the 「이스케이프하지 않는다」 (does not escape) in the doc above).
    #[test]
    fn console_command_wraps_only_on_windows_and_keeps_args_verbatim() {
        let args = vec![
            "--flag".to_string(),
            "a b".to_string(),
            "%HOME%&x".to_string(),
            String::new(),
        ];
        let (program, wrapped) = console_command("cli", args.clone());
        if cfg!(windows) {
            assert_eq!(program, "cmd.exe");
            let mut expected = vec!["/c".to_string(), "cli".to_string()];
            expected.extend(args);
            assert_eq!(wrapped, expected);
        } else {
            assert_eq!(program, "cli");
            assert_eq!(wrapped, args);
        }
    }
}
