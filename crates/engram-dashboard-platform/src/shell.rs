//! OS rules for launching console programs — the default interactive shell · wrapping a CLI in that OS's console shell. Uses only std.

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
///   The caller mitigates per value — e.g. daemon `control/priming.rs`'s `CMD_UNSAFE_CHARS` · `path_is_cli_safe`, codex's
///   `developer_instructions_override` · `mcp_attachment` rejection, claude `--settings` as a file path instead of inline JSON.
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

#[cfg(test)]
mod tests {
    use super::*;

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
