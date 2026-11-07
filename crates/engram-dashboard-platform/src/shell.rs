//! OS rules for launching console programs — the default interactive shell · wrapping a CLI in that OS's console shell. Uses only std.

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
///   지나며 치환된다」 inside `build_spec` in agent `backend/codex/mod.rs`. Mitigation is placed by the caller per value — e.g. daemon `control/priming.rs`'s
///   `CMD_UNSAFE_CHARS` · `path_is_cli_safe` · codex's `developer_instructions_override` · `mcp_attachment`
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

#[cfg(test)]
mod tests {
    use super::*;

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
