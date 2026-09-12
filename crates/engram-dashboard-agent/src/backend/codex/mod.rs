//! CodexBackend — codex CLI 전용 CommandSpec 산출.
//!
//! ★이 폴더가 세우는 규칙 = codex 지식은 여기 안에만 산다(ADR-0004)★. 근거·게이트·게이트가
//! 못 보는 것의 정본은 `backend/claude/mod.rs` 헤더이고 여기 되풀어 적지 않는다 — 이름만 바꿔
//! 읽는다. 밖으로 나가는 표면은 [`crate::backend::AgentBackend`] 구현 하나뿐이다.
//!
//! ★여기 적힌 codex 사실은 실측이다(codex-cli 0.153.4, 이 PC, 인증됨 — 2026-09-08 재확인)★.
//! `tests/backend_contract.rs` 가 이 파일의 `build_spec` 이 낸 argv 를 **그대로 띄우므로**, 여기서 인자를
//! 바꾸면 그 레인이 바뀐 argv 로 실 codex 를 겪는다 — ★단 그 레인은 `#[ignore]` 라 부를 때만 돈다(CI 아님)★.
//!
//! ★시험대가 **다시 재지 않는 것** — 「전부 실측」이라 적던 옛 문장이 거짓이었다(리뷰 적출 2026-09-08)★:
//!   1. `workspace-write`·`on-request` 정책 **아래의 모델 동작** — 시험대는 그 argv 로 뜨는 것과 컴포저
//!      기립까지만 잰다(재려면 쓰기 권한을 가진 에이전트를 자동 레인에서 실제로 돌려야 한다).
//!   2. 우리가 안 쓰는 인자(`-m` · MCP `-c mcp_servers.…` 오버라이드 문법 · `codex resume <id>`) — 이
//!      파일 주석에만 있고 재는 곳이 없다.
//!   3. 아래 `build_spec` 의 `%VAR%` 한계(그 자리 주석이 정본).
//!
//! capability 선언이 그 표와 어긋나면 시험대의 **비-`#[ignore]`** 항목이 빨개진다.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::backend::{console_command, AgentBackend};
use crate::profile::{AgentCommand, SpawnMode};
use crate::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

/// PATH 로 해석되는 이름 그대로 띄운다(사용자 결정 2026-09-07 · TRD §6-H).
///
/// ★실 바이너리를 찾아 직접 띄우지 않는 이유★: Windows 에서 PATH 의 `codex` 는
/// `codex.cmd → node → codex.exe` 사슬이고 그 끝의 실 바이너리는 **버전이 박힌 `node_modules` 벤더
/// 경로** 아래 있다(실측). codex 는 스스로 자동 업데이트하므로 그 경로는 우리가 모르는 시점에 바뀐다 —
/// 하드코딩하면 업데이트 한 번에 죽고, 탐색·폴백을 짜면 그 사슬을 우리가 재구현하게 된다.
/// ★한 겹 더 깊은 shim 이 kill 인과를 바꾸지 않는다★: Job Object 가 트리를 통째로 끝내므로
/// (ADR-0001 의 2 동사) 손자·증손자까지 함께 내려간다(실측).
const CODEX_PROGRAM: &str = "codex";

/// codex 가 작업 폴더를 받는 플래그. ★프로세스 cwd 와 별개다★ — `CommandSpec.cwd` 는 우리가 프로세스를
/// 어디서 띄우나이고, 이 값은 codex 가 **어느 폴더를 워크스페이스로 신뢰·편집하나**다. 둘을 같은 값으로
/// 주지만 같은 칸이 아니다.
const CD_FLAG: &str = "--cd";

/// 샌드박스 모드 — 모델이 내는 명령을 **작업 폴더 안에서는 쓰기까지** 허용한다.
const SANDBOX_FLAG: &str = "-s";
const SANDBOX_WORKSPACE_WRITE: &str = "workspace-write";

/// 승인 모드 — 샌드박스 밖으로 나가는 동작만 사람에게 묻는다.
const APPROVAL_FLAG: &str = "-a";
const APPROVAL_ON_REQUEST: &str = "on-request";

pub struct CodexBackend;

impl AgentBackend for CodexBackend {
    /// ★호출자가 세션 id 를 정할 수 없다(실측)★ — codex 에는 `--session-id` 류 플래그가 없고 id 는
    /// codex 가 스스로 발급한다. true 로 두면 manager 가 우리 uuid 를 발급해 추적기를 붙이는데, 그 값은
    /// codex 가 쓰지 않으므로 영영 나타나지 않을 파일을 폴링하게 된다.
    fn needs_session(&self) -> bool {
        false
    }

    fn supports_control_channel(&self) -> bool {
        false
    }

    /// ★"codex 가 MCP 를 못 쓴다" 가 아니다★ — 이 칸이 묻는 것은 **우리가 만든 mcp-config 파일을 먹일 수
    /// 있나**이고 그 답이 아니오다. codex 의 MCP 주입은 전역 TOML 오버라이드
    /// (`-c mcp_servers.<name>={…}`)라 claude 의 `--mcp-config <path>` 와 **기제가 다르다**(실측).
    /// 그 다른 기제를 배선하는 것은 이 단계의 범위가 아니다.
    // ADR-0099
    fn accepts_mcp_config(&self) -> bool {
        false
    }

    /// ★분류 사유가 shell 과 다르다★ — shell 이 false 인 것은 입력이 **명령으로 실행되기** 때문이고,
    /// codex 가 false 인 것은 **바쁜 때를 못 가리기** 때문이다. 이 backend 는 턴 신호를 하나도 선언하지
    /// 않는데(터미널 모드엔 구조화 이벤트가 없다) 바쁨 게이트가 fail-open 이라, 선언 없는 백엔드는 늘
    /// 한가한 것으로 읽혀 **생각하는 도중에 편지가 꽂힌다**. 그래서 수신자 명단에서 아예 뺀다.
    /// ★여는 조건도 다르다★: 턴을 관측할 수 있게 되면(상주 JSON 서버) 이 값이 열린다 — shell 쪽 사유는
    /// 그때도 그대로 남으므로 둘을 같이 열지 말 것.
    fn reads_messages(&self) -> bool {
        false
    }

    /// ★세션 인자를 조립하지 않는다★ — `--session-id` 는 존재하지 않고, 재개는 플래그가 아니라 하위
    /// 명령 + 위치 인자(`codex resume <id>`)라 이 자리의 문법이 아니다(실측). `session_id` 는
    /// `needs_session()` 이 false 라 항상 `None` 이지만, 계약상 받는 값이므로 무시한다는 것을 적어 둔다.
    // ADR-0004
    fn build_spec(
        &self,
        command: &AgentCommand,
        _mode: SpawnMode,
        _session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        _control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        match command {
            AgentCommand::Codex { extra_args, .. } => {
                let mut args = Vec::with_capacity(6 + extra_args.len());
                args.push(CD_FLAG.to_string());
                args.push(cwd.to_string_lossy().into_owned());
                args.push(SANDBOX_FLAG.to_string());
                args.push(SANDBOX_WORKSPACE_WRITE.to_string());
                args.push(APPROVAL_FLAG.to_string());
                args.push(APPROVAL_ON_REQUEST.to_string());
                // 우리 인자를 먼저 소진하고 호출자 패스스루를 뒤에 잇는다 — 위 셋은 전부 값 하나짜리라
                //   뒤 인자를 흡수하지 않는다(claude 의 variadic `--allowedTools` 와 다른 점).
                args.extend(extra_args.iter().cloned());
                // ★알려진 한계 — `%VAR%` 가 든 경로는 shim 을 지나며 치환된다(2026-09-08, 고치지 않기로
                //   한 결정)★. 아래 `console_command` 가 Windows 에서 `cmd.exe /c` 로 감싸는데, cmd 는
                //   명령줄의 `%NAME%` 을 **따옴표 안에서도** 환경변수로 편다. 그래서 이름에 `%…%` 가
                //   들어간 실제 폴더(`C:\x\%USERNAME%\y`)를 받으면 codex 는 **다른 폴더**를 워크스페이스로
                //   본다. 위 `CD_FLAG` 값과 `extra_args` 가 그 경로를 탄다.
                //   ★왜 안 고치나★ — ① shim 은 걷을 수 없다(사용자 결정 · TRD §6-H: `codex` 를 이름
                //   그대로 띄운다. PATH 의 `codex` 는 `.cmd` 라 `CreateProcessW` 가 직접 못 띄운다)
                //   ② 알려진 해법이 **일반적인 이스케이프가 아니다**: rust std 가 `.bat` 을 띄울 때 쓰는
                //   수법은 `%` 마다 `%%cd:~,%`(no-op 치환)를 끼워 넣는 것이고 `cmd /e:ON` + `/c "한 문자열"`
                //   형태를 전제한다(RUSTSEC-2024-0006 대응). 우리 경로는 그 형태가 아니고(인자 배열을
                //   portable-pty 가 직접 조립해 `CreateProcessW` 로 넘긴다 — `%` 처리 0줄), 그 수법을 그
                //   전제 밖으로 옮기는 것은 검증되지 않은 이식이다 ③ 그 이식은 **claude 와 공용인**
                //   `console_command` 를 건드리므로, 틀리면 오늘 도는 스폰 전부가 죽는다.
                //   ★고칠 때 무엇이 필요한가★: 위 셋을 뒤집을 실측(그 형태로 감싼 `cmd` 아래에서 claude·
                //   codex 가 둘 다 뜨는가)이다. 그 전에는 이 한계를 아는 채로 둔다.
                // ADR-0004
                let (program, args) = console_command(CODEX_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            AgentCommand::Claude { .. } | AgentCommand::Shell { .. } => {
                unreachable!("CodexBackend 는 Codex variant 만 처리한다. dispatch 버그.")
            }
        }
    }

    /// `session.resume = false` 인 이유는 "아직 안 재 봤다" 가 아니라 **호출자가 sid 를 못 정하므로
    /// 무손실 복원이 성립하지 않는다**는 것이다(실측 — `needs_session` 참조).
    /// `model.select` 는 codex 에 `-m` 이 있는데도 false 다 — 이 칸은 **그 프로그램이 할 수 있는 것**이
    /// 아니라 **이 스폰이 쓰는 것**을 신고한다. 그 칸을 노출하지 않으므로 신고하지 않는다.
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: false,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AgentOutputFormat;

    fn codex(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn spec(command: &AgentCommand, cwd: &str) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            PathBuf::from(cwd),
            vec![],
            None,
        )
    }

    /// `console_command` 래핑을 걷어 낸 codex 자신의 argv. Windows 는 `cmd.exe /c codex …` 로 한 겹
    /// 감싸이므로(shim 해석) 두 플랫폼의 단언을 하나로 쓰려면 그 겹을 여기서 벗긴다.
    fn codex_argv(spec: &CommandSpec) -> Vec<String> {
        #[cfg(windows)]
        {
            assert_eq!(spec.program, "cmd.exe");
            assert_eq!(spec.args[0], "/c");
            assert_eq!(spec.args[1], CODEX_PROGRAM);
            spec.args[2..].to_vec()
        }
        #[cfg(not(windows))]
        {
            assert_eq!(spec.program, CODEX_PROGRAM);
            spec.args.clone()
        }
    }

    #[test]
    fn interactive_args_are_the_measured_ones() {
        let s = spec(&codex(vec![]), "C:/workspace");
        assert_eq!(
            codex_argv(&s),
            vec![
                "--cd",
                "C:/workspace",
                "-s",
                "workspace-write",
                "-a",
                "on-request"
            ]
        );
    }

    #[test]
    fn extra_args_come_last() {
        let s = spec(&codex(vec!["-m", "gpt-5"]), "C:/workspace");
        let argv = codex_argv(&s);
        assert_eq!(
            &argv[argv.len() - 2..],
            &["-m".to_string(), "gpt-5".to_string()]
        );
    }

    /// ★세션 인자가 조립되면 안 된다★ — 존재하지 않는 문법이라 붙는 순간 codex 가 기동에 실패한다.
    #[test]
    fn session_id_never_reaches_the_command_line() {
        let sid = Uuid::new_v4();
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            Some(sid),
            PathBuf::from("."),
            vec![],
            None,
        );
        assert!(
            !s.args.iter().any(|a| a.contains(&sid.to_string())),
            "sid 가 argv 에 실렸다: {:?}",
            s.args
        );
        assert!(
            !s.args
                .iter()
                .any(|a| a == "--session-id" || a == "--resume" || a == "--session"),
            "존재하지 않는 세션 플래그가 실렸다: {:?}",
            s.args
        );
    }

    #[test]
    fn needs_session_is_false() {
        assert!(!CodexBackend.needs_session());
    }

    #[test]
    fn reads_messages_is_false() {
        assert!(!CodexBackend.reads_messages());
    }

    #[test]
    fn capabilities_resume_is_false() {
        assert!(!CodexBackend.capabilities(&codex(vec![])).session.resume);
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("BAR".to_string(), "baz".to_string())];
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Fresh,
            None,
            cwd.clone(),
            env.clone(),
            None,
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
