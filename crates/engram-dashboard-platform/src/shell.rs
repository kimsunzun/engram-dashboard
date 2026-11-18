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

/// Turns one CLI program into the `(program, args)` to launch — Windows = `("cmd.exe", ["/c", program, args…])`, elsewhere = as received.
/// **Why wrap:** an npm-installed CLI on Windows is an extensionless shim (+ `.cmd`), which `CreateProcessW` (ConPTY uses it too) cannot
///   launch directly (error 193 — no PATHEXT or shell interpretation); `cmd.exe /c <prog> …` lets cmd resolve the
/// `<prog>.cmd` shim. `cmd /c` ends with its target, so "launched child = that CLI" lifetime holds (killing the whole tree is the Job Object's).
/// - **CLI (shim) only** — not for an executable launchable directly (`cmd.exe` itself and the like).
/// - **Known limitation — arguments are not escaped by cmd rules.** On Windows arguments ride as is on `cmd.exe`'s command line, so an
///   argument with cmd metacharacters (`%NAME%` — expanded even inside quotes · `& ^ | < >`) or the `"` and `\` cmd
///   counts as quotes may arrive changed. The decision not to fix it and its rationale are canonical in the 「Known limit — a path
///   containing `%VAR%` gets substituted while passing the shim」 comment in `build_spec` of agent `backend/codex/mod.rs`.
///   The caller mitigates per value — e.g. [`path_is_cli_safe`] · [`text_is_cli_safe`], codex's `cmdline::priming_value` ·
///   `mcp_attachment` rejection, claude `--settings` as a file path instead of inline JSON.
/// - A process launched with this command is, on Windows, the `cmd.exe` wrapper, not the CLI — the CLI is a descendant under it.
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
            "C:/base/builtin/instructions/priming.md"
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
            "한글", // hangul-ok — non-ASCII text is safe
        ] {
            assert!(text_is_cli_safe(text), "{text:?}");
        }
    }

    /// ★Arguments ride in order without changing a character★ — arguments with spaces or cmd metacharacters get no quotes or escapes added
    ///   (measures the "does not escape" above).
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
