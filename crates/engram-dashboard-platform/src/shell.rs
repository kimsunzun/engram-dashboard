//! 콘솔 프로그램을 띄우는 OS 규칙 — 대화형 기본 셸 · CLI 를 그 OS 의 콘솔 셸로 감싸기. std 만 쓴다.

#[cfg(windows)]
pub fn default_shell() -> &'static str {
    "cmd.exe"
}
#[cfg(not(windows))]
pub fn default_shell() -> &'static str {
    "bash"
}

/// CLI 프로그램 하나를 띄울 `(program, args)` 로 바꾼다 — Windows = `("cmd.exe", ["/c", program, args…])`,
/// 그 밖 = 받은 그대로.
///
/// **왜 감싸나:** Windows 에서 npm 이 까는 CLI 는 확장자 없는 shim(+ `.cmd`)이라 `CreateProcessW`(ConPTY 도
/// 이것을 쓴다)가 직접 못 띄운다(error 193 — PATHEXT 도 셸 해석도 안 한다). `cmd.exe /c <prog> …` 로 감싸면
/// cmd 가 `<prog>.cmd` shim 을 해석해 실제 프로세스를 띄운다. `cmd /c` 는 대상이 끝나면 함께 끝나므로
/// 「띄운 자식 = 그 CLI」 수명이 유지된다(트리째 끄기는 Job Object 가 한다).
///
/// - **CLI(shim) 전용이다** — 직접 띄울 수 있는 실행 파일(`cmd.exe` 자신 따위)에는 쓰지 않는다.
/// - **알려진 한계 — 인자를 cmd 규칙으로 이스케이프하지 않는다.** Windows 에서 인자는 그대로 `cmd.exe` 의
///   명령줄에 실리고, cmd 메타문자(`%NAME%` — 따옴표 안에서도 환경변수로 펼친다 · `& ^ | < >`)나 cmd 가
///   따옴표를 세는 `"` · `\` 가 든 인자는 cmd 가 해석해 바뀐 채 닿을 수 있다. 고치지 않기로 한 결정과 그
///   근거는 agent `backend/codex/mod.rs` 의 `build_spec` 안 「알려진 한계 — `%VAR%` 가 든 경로는 shim 을
///   지나며 치환된다」 주석이 정본이다. 완화는 부르는 쪽이 값마다 둔다 — 예: daemon `control/priming.rs` 의
///   `CMD_UNSAFE_CHARS` · `path_is_cli_safe` · codex 의 `developer_instructions_override` · `mcp_attachment`
///   거절 · claude `--settings` 를 인라인 JSON 대신 파일 경로로 넘기기.
/// - 이 명령으로 띄운 프로세스는 Windows 에서 그 CLI 가 아니라 `cmd.exe` 래퍼다 — CLI 자신은 그 아래 후손이다.
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

    /// ★인자는 한 글자도 바뀌지 않고 순서대로 실린다★ — 공백 · cmd 메타문자가 든 인자도 따옴표나 이스케이프가
    /// 덧붙지 않는다(위 doc 의 「이스케이프하지 않는다」를 잰다).
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
