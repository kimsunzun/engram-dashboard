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

pub(crate) mod decoder;
pub(crate) mod protocol;
pub(crate) mod transport;

use std::path::PathBuf;

use uuid::Uuid;

use self::decoder::CodexAppServerDecoder;
use self::protocol::{AskForApproval, SandboxMode, ThreadStartParams};
use self::transport::CodexAppServerTransport;
use crate::backend::{
    console_command, AgentBackend, InputEncoder, SessionIdSink, SpawnParts, TransportShape,
    TurnClassifier,
};
use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, OutputDecoder};
use crate::turn::TurnSignal;
use crate::types::{
    BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, OutputEvent, PtyError, SessionCaps,
};

/// codex 를 대화형 TUI 가 아니라 **상주 JSON 서버**로 띄우나 = 이 폴더 안의 네 축(통로 모양·통로 실물·
/// 입력 인코딩·출력 decoder)이 함께 갈리는 지점.
///
/// ★이 술어를 `profile` 로 올리지 말 것(ADR-0004)★: 거기 있으면 codex 의 출력 형식 축이 공용 층의 통로
///   선택을 직접 굴린다. 밖으로 나가는 것은 중립 축의 값뿐이다.
fn is_app_server(command: &AgentCommand) -> bool {
    matches!(
        command,
        AgentCommand::Codex {
            output_format: AgentOutputFormat::StreamJson,
            ..
        }
    )
}

/// PATH 로 해석되는 이름 그대로 띄운다(사용자 결정 2026-09-07 · TRD §6-H).
///
/// ★실 바이너리를 찾아 직접 띄우지 않는 이유★: Windows 에서 PATH 의 `codex` 는
/// `codex.cmd → node → codex.exe` 사슬이고 그 끝의 실 바이너리는 **버전이 박힌 `node_modules` 벤더
/// 경로** 아래 있다(실측). codex 는 스스로 자동 업데이트하므로 그 경로는 우리가 모르는 시점에 바뀐다 —
/// 하드코딩하면 업데이트 한 번에 죽고, 탐색·폴백을 짜면 그 사슬을 우리가 재구현하게 된다.
/// ★한 겹 더 깊은 shim 이 kill 인과를 바꾸지 않는다 — 단 그 보장의 범위를 정확히 적는다★:
/// Job 에 **편입된 뒤로는** breakaway 가 막혀 있어(`BREAKAWAY_OK`·`SILENT_BREAKAWAY_OK` 둘 다 안 켠다)
/// 그 아래 생기는 손자·증손자가 Job 을 벗어날 수 없고, `TerminateJobObject` 가 트리를 통째로 끝낸다
/// (ADR-0001 의 2 동사).
/// ★편입은 spawn **뒤**라 그 사이 창은 그 보장 밖이다★ — 그 창에서 태어난 자손은 Job 에 안 들어간다.
/// 이 저장소의 통로 셋이 전부 같은 모양이고(`CREATE_SUSPENDED` 는 한 줄도 없다) 기존 teardown 테스트는
/// 정착 상태만 재므로, 이 창은 **재 본 적이 없다**. 고치는 것은 세 통로를 함께 건드리는 별건이다.
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

/// 상주 JSON 서버로 띄우는 하위 명령과 그 전송 선택(실측 0.154.0 — `--stdio` 는 `--listen stdio://` 와
/// 같고 그것이 기본값이다. 기본값에 기대지 않고 명시한다: 이 통로는 stdio 가 아니면 성립하지 않는데,
/// 기본값은 상류가 바꿀 수 있고 바뀌어도 우리 argv 는 조용히 그대로다).
const APP_SERVER_SUBCOMMAND: &str = "app-server";
const APP_SERVER_STDIO_FLAG: &str = "--stdio";

pub struct CodexBackend;

impl AgentBackend for CodexBackend {
    /// ★호출자가 세션 id 를 정할 수 없다(실측)★ — codex 에는 `--session-id` 류 플래그가 없고 id 는
    /// codex 가 스스로 발급한다. true 로 두면 manager 가 우리 uuid 를 발급해 **프로필에 영속**하는데 그
    /// 값은 codex 가 한 번도 쓰지 않는다 — 그러고 나면 이어받기 판정이 그 가짜 값을 보고 서서, 실제로는
    /// 새 대화인 화신을 이어받았다고 믿는다.
    /// ★app-server 모드라고 켜지 말 것 — 두 모양 다 false 다★: 그 모드의 식별자(thread id)도 발급 주체는
    ///   codex 다. 그 id 로 **이어받을 수 있나**는 아래 별개 축이 답한다.
    // ADR-0185
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        false
    }

    /// ★터미널 모드에는 이어받을 식별자 자체가 없고, app-server 모드에는 있다(thread id)★ — 그런데도 두
    /// 모양 다 false 인 것은 **선언과 실물을 같게 두기 위해서**다.
    /// ★`is_app_server(command)` 로 바꾸는 것은 `thread/resume` 을 배선하는 그 커밋이다 — 그 전에 켜지 말
    ///   것★: 지금 [`AgentBackend::build_spec`] 은 mode·sid 를 무시하고 [`AgentBackend::open_spawn`] 은
    ///   언제나 `thread/start` 를 낸다. 이 칸만 먼저 true 로 두면, 이 백엔드에 sid 가 생기는 순간(슬라이스
    ///   2 가 그것을 배선하고, 손으로 고친 `agents.json` 은 오늘도 그 상태를 만든다) 복원·활성화가 Resume
    ///   으로 가고 **새 스레드가 열리는데 결과는 「이어받음」으로 보고된다**. 지금은 그 입력이 Fresh 로
    ///   떨어져 정직하게 보고된다.
    // ADR-0185
    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
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
    /// codex 가 false 인 것은 **바쁜 때를 못 가리기** 때문이다. 바쁨 게이트는 fail-open 이라, 턴 신호가
    /// 없는 백엔드는 늘 한가한 것으로 읽혀 **생각하는 도중에 편지가 꽂힌다**.
    /// ★app-server 모드는 이제 그 신호를 낸다★ — 통로가 `turn/completed` 를 읽고(`turn/started` 는
    /// **일부러** 읽지 않는다 — 사유 정본은 그 통로의 `TURN_COMPLETED` doc), 번역기가 같은 알림을 턴
    /// 경계로 옮겨 아래 [`classify_turn`] 이 `Ended` 를 낸다. ★그런데도 이 칸이 false 인 것은 **터미널
    /// 모드 때문**이다★: 이 메서드는 `command` 를 받지 않아 두 모드를 가를 수 없는데, 그 모드는 decoder
    /// 가 없어 `TerminalBytes` 만 흐르고 그래서 신호가 하나도 없다. 여기서 true 를 돌려주면 그 모드까지
    /// 함께 열린다.
    /// ★여는 조건 = 이 축을 모드별로 가르는 것★(시그니처에 명령을 들이거나 자격을 세션 caps 로 옮기거나).
    /// shell 쪽 사유는 그때도 그대로 남으므로 둘을 같이 열지 말 것.
    fn reads_messages(&self) -> bool {
        false
    }

    /// ★세션 인자를 조립하지 않는다★ — `--session-id` 는 존재하지 않고, 재개는 플래그가 아니라 하위
    /// 명령 + 위치 인자(`codex resume <id>`)라 이 자리의 문법이 아니다(실측). `session_id` 는
    /// `assigns_session_id()` 가 false 라 항상 `None` 이지만, 계약상 받는 값이므로 무시한다는
    /// 것을 적어 둔다.
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
            AgentCommand::Codex {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(6 + extra_args.len());
                match output_format {
                    AgentOutputFormat::Terminal => {
                        args.push(CD_FLAG.to_string());
                        args.push(cwd.to_string_lossy().into_owned());
                        args.push(SANDBOX_FLAG.to_string());
                        args.push(SANDBOX_WORKSPACE_WRITE.to_string());
                        args.push(APPROVAL_FLAG.to_string());
                        args.push(APPROVAL_ON_REQUEST.to_string());
                    }
                    // ★정책과 작업 폴더가 여기 안 실리는 것은 의도다★ — app-server 는 그 셋을
                    //   `thread/start` 파라미터(`cwd`·`sandbox`·`approvalPolicy`)로 받는다(실측 0.154.0).
                    //   그 값을 만드는 자리는 아래 [`AgentBackend::open_spawn`] 이고, 프로세스 cwd 는
                    //   `CommandSpec.cwd` 로 남는다 — 둘은 같은 값을 받지만 같은 칸이 아니다.
                    AgentOutputFormat::StreamJson => {
                        args.push(APP_SERVER_SUBCOMMAND.to_string());
                        args.push(APP_SERVER_STDIO_FLAG.to_string());
                    }
                }
                // 우리 인자를 먼저 소진하고 호출자 패스스루를 뒤에 잇는다 — 터미널 모드의 셋은 전부 값
                //   하나짜리라 뒤 인자를 흡수하지 않는다(claude 의 variadic `--allowedTools` 와 다른 점).
                // ★패스스루는 모드를 안 가린다 — 그리고 두 모드의 옵션 집합은 다르다(실측 0.154.0:
                //   `codex --help` 와 `codex app-server --help` 가 서로 없는 플래그를 갖는다)★. 즉 대화형
                //   CLI 를 보고 적은 인자는 app-server 에서 거절될 수 있다. 그래도 모드별로 거르지 않는다:
                //   무엇이 유효한지는 상류가 버전마다 바꾸므로 우리가 든 명단은 낡고, 낡은 명단은 멀쩡한
                //   인자를 조용히 지운다.
                args.extend(extra_args.iter().cloned());
                // ★알려진 한계 — `%VAR%` 가 든 경로는 shim 을 지나며 치환된다(2026-09-08, 고치지 않기로
                //   한 결정)★. 아래 `console_command` 가 Windows 에서 `cmd.exe /c` 로 감싸는데, cmd 는
                //   명령줄의 `%NAME%` 을 **따옴표 안에서도** 환경변수로 편다. 그래서 이름에 `%…%` 가
                //   들어간 실제 폴더(`C:\x\%USERNAME%\y`)를 받으면 codex 는 **다른 폴더**를 워크스페이스로
                //   본다. 터미널 모드에서는 위 `CD_FLAG` 값과 `extra_args` 가, app-server 모드에서는
                //   `extra_args` 만 그 경로를 탄다(그 모드의 작업 폴더는 명령줄이 아니라 `thread/start`
                //   파라미터로 가므로 cmd 를 지나지 않는다).
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

    /// `session.resume = false` 인 이유는 ★**발급 주체와 무관하다**★ — 복원은 프로필에 저장된 backend
    /// sid **단독**에 의존하고 그 sid 를 누가 발급하는지는 백엔드가 정한다. codex 는 `thread/start`
    /// 응답으로 받아 쓰는 쪽이다.
    /// ★**「받아 적는 배선이 없어서」는 낡은 사유다 — 그 배선은 섰다**★: [`AgentBackend::open_spawn`] 이
    /// 조립점의 기록 동사를 받아 app-server 통로에 넘기고, 그 통로가 `thread/start` 응답의 id 로 그것을
    /// 부른다(실 app-server 실측 — 받은 id 가 프로필에 앉는다). 지금 이 칸이 false 인 사유는 **하나
    /// 남았다 — `thread/resume` 을 내지 않는다**. 받은 id 는
    /// 프로필에 적히기만 하고 이어받기에 쓰이지 않는다.
    /// ★판정 축([`AgentBackend::can_resume_stored_session`])도 **아직 꺼져 있고, 그 축과 이 칸은 여전히
    /// 같은 하나를 기다린다**★ — 이제 그 하나는 「수령 배선」이 아니라 `thread/resume` 이다. 둘 중 하나만
    /// 먼저 켜지 말 것: 어느 쪽이든 단독으로 켜면 이어받은 적 없는 새 스레드가 「이어받음」으로 보고된다.
    /// `model.select` 는 codex 에 `-m` 이 있는데도 false 다 — 이 칸은 **그 프로그램이 할 수 있는 것**이
    /// 아니라 **이 스폰이 쓰는 것**을 신고한다. 그 칸을 노출하지 않으므로 신고하지 않는다.
    // ADR-0185
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

    /// app-server 모드는 봉투가 **양방향**으로 흐르는 파이프를 요구한다 — 상대가 우리에게도 묻는다.
    ///
    /// ★신고값일 뿐이고 실물은 아래 [`AgentBackend::open_spawn`] 이 만든다★ — 둘 다 [`is_app_server`] 를
    ///   보므로 한쪽만 고치면 신고와 실물이 어긋난다.
    fn transport_shape(&self, command: &AgentCommand) -> TransportShape {
        if is_app_server(command) {
            TransportShape::StdioBidiJson
        } else {
            TransportShape::Pty
        }
    }

    /// app-server 모드는 양방향 JSON 통로를, 터미널 모드는 PTY 를 만든다.
    ///
    /// ★판정은 [`is_app_server`] 단독 — `transport_shape` 신고값을 되읽어 match 하지 않는다(ADR-0191)★:
    ///   그렇게 하면 모양 값을 가르는 둘째 switch 가 생긴다.
    /// ★`structured: true` 와 `thread/start` 정책을 주입하는 자리가 여기다(ADR-0044/0030)★: 통로는 자기가
    ///   나르는 바이트가 무엇인지도, 어느 폴더를 어떤 샌드박스로 열어야 하는지도 모른다. 아는 쪽은 이
    ///   모드와 정책을 고른 이 backend 다.
    /// ★`sid_sink` 를 app-server 갈래에만 넘긴다★ — 터미널 갈래에는 받아 올 식별자 자체가 없다. 그
    ///   포트로 나가는 값은 codex 가 `thread/start` 응답으로 발급한 thread id 이고, 통로가 그 세션으로
    ///   무엇을 보내기 전에 나간다([`AgentBackend::open_spawn`] 의 순서 계약).
    /// ★그 배선이 섰다고 `capabilities().session.resume` 이나
    ///   [`AgentBackend::can_resume_stored_session`] 이 함께 켜지는 것이 아니다★ — 그 둘은 `thread/resume`
    ///   을 실제로 내는 커밋이 함께 켠다. 여기까지는 값이 프로필에 **남기만** 한다.
    // ADR-0185
    // ADR-0191
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
    ) -> Result<SpawnParts, PtyError> {
        let (transport, child_pid): (Box<dyn AgentTransport>, Option<u32>) =
            if is_app_server(command) {
                let params = ThreadStartParams {
                    cwd: Some(spec.cwd.to_string_lossy().into_owned()),
                    approval_policy: Some(AskForApproval::OnRequest),
                    sandbox: Some(SandboxMode::WorkspaceWrite),
                };
                let (t, pid) = CodexAppServerTransport::open(
                    spec,
                    true,
                    self.output_decoder(command),
                    params,
                    sid_sink,
                )?;
                (Box::new(t), pid)
            } else {
                let (t, pid) = PtyTransport::open(spec, cols, rows)?;
                (Box::new(t), pid)
            };
        Ok(SpawnParts {
            transport,
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
        })
    }

    /// app-server 모드에서는 봉투를 **통로가** 만든다 — 인코더가 감쌀 것이 없다.
    fn input_encoder(&self, command: &AgentCommand) -> InputEncoder {
        if is_app_server(command) {
            InputEncoder::TransportFramed
        } else {
            InputEncoder::Raw
        }
    }

    fn turn_classifier(&self) -> TurnClassifier {
        classify_turn
    }

    // ★합성 입력 에코를 선언하지 않는다 — 되살리지 말 것★: 이 백엔드는 유저 메시지를 스스로
    //   `item/*` 의 `userMessage` item 으로 되울리고(실측), 번역기가 그것을 「우리가 보낸 것」으로
    //   표시해 흘린다. 여기에 합성 에코를 더하면 같은 질문이 화면에 두 벌 남는다 — 둘의 dedup 키가
    //   다르기 때문이다(합성 쪽은 우리 uuid, 되울린 쪽은 codex item id). 그 겹침이 ADR-0193 의
    //   「거부한 대안」이 실측을 근거로 기각한 바로 그것이다.
    // ADR-0193

    /// ★선언하지 않으면 번역기가 조립되지 않고 바이트가 그대로 흘러 화면이 깨진다 — 오류도 경고도 없다★.
    fn output_decoder(&self, command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        if is_app_server(command) {
            Some(Box::new(CodexAppServerDecoder::new()))
        } else {
            None
        }
    }
}

/// codex 출력 이벤트 → 턴 신호(ADR-0113).
///
/// ★이 백엔드의 종료 신호는 [`OutputEvent::TurnEnd`] 다★ — 번역기는 `MessageDone` 을 내지 않는다.
///   그래도 그 갈래를 `Ended` 로 적어 둔다: 뜻이 같은 두 어휘를 여기서 갈라 적으면, 어느 날 이 백엔드가
///   그것을 내게 됐을 때 종료가 조용히 사라진다.
/// ★`Structured` 를 진행으로 세는 것이 이 백엔드에서 load-bearing 이다★: 이 갈래로 들어오는 것은
///   되울린 유저 메시지(`userMessage` item 번역분) 하나뿐이고 — 모르는 알림은 버리므로(이 폴더
///   `decoder` 헤더) 그 밖엔 없다 — 그것이 「이 턴이 시작됐다」의 첫 관측이다.
/// ★그래도 「우리가 `turn/start` 를 쓴 순간부터 그 첫 알림이 도착하기까지」는 미관측이다★ — 그 창을
///   합성 에코로 메우지 않는다(ADR-0193 이 그 대안을 기각했다). 큐 해제의 「지금 보낼 수 있나」는 통로
///   구현체가 자기 상태로 답하고, 우편 게이트 쪽 미관측은 fail-open 으로 흡수된다(ADR-0104 의 선택).
/// ★`Usage`/`Error` 가 종료가 아닌 이유★: `Usage`(`thread/tokenUsage/updated`)는 턴 중간에 오고,
///   `Error`(`error` 알림)는 재시도 가능한 스트림 오류라 턴 경계가 아니다. 실패한 턴도 `turn/completed`
///   가 내는 `TurnEnd` 로 닫힌다 — 실패 사유는 그 이벤트 **안에** 실린다.
/// ★`turn/completed` 가 **아예 오지 않는** 포기 경로도 `TurnEnd` 로 닫힌다★(쓰기 실패·응답 해독 실패
///   ·오류 응답·시한 만료 — 통로의 `end_turn_if`). 그 자리에 `Error` 를 쓰던 시절에는 이 표가 종료를
///   못 세서 **한가한 에이전트가 턴 중으로 관측된 채 남아** 30 분 fail-open 밸브까지 우편이 막혔다
///   (ADR-0127). 그래서 이 매핑에서 `Error` 를 종료로 승격시키는 것이 아니라, 그쪽이 경계 어휘를 쓴다.
/// ★터미널 모드와 공유해도 되는 이유(모드별 분기 불필요)★: 그 모드는 decoder 가 없어 `TerminalBytes` 만
///   흐르므로 이 매핑을 그대로 써도 신호가 하나도 나오지 않는다.
/// ★`Ended` 앞에 `Progress` 가 없어도 안전하다(코드 근거)★: 표는 `Ended` 를 `in_turn = false` 로 적을
///   뿐이라 짝 없는 종료는 등록 직후 상태와 같은 값을 쓰고(`crate::turn::TurnObservations::observe_at`),
///   `in_turn_snapshot` 은 `in_turn` 인 것만 싣는다. 그래서 「시작 신호」를 지어내 채울 이유가 없다.
// ADR-0113
// ADR-0004
pub(crate) fn classify_turn(event: &OutputEvent) -> Option<TurnSignal> {
    match event {
        OutputEvent::TextDelta { .. }
        | OutputEvent::ToolCall { .. }
        | OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
        OutputEvent::TurnEnd { .. } | OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended),
        OutputEvent::Usage { .. } | OutputEvent::Error(_) | OutputEvent::TerminalBytes(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TurnOutcome;

    fn codex(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn codex_app_server(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::StreamJson,
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
    fn app_server_args_are_the_measured_ones() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// ★정책과 작업 폴더는 argv 로 가지 않는다★ — app-server 는 그 셋을 `thread/start` 파라미터로 받는다.
    /// 여기 실리면 상대가 그 하위 명령에서 모르는 인자를 받아 기동이 실패한다.
    #[test]
    fn app_server_argv_carries_no_policy_and_no_workspace_path() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        let argv = codex_argv(&s);
        for forbidden in [CD_FLAG, SANDBOX_FLAG, APPROVAL_FLAG, "C:/workspace"] {
            assert!(
                !argv.iter().any(|a| a == forbidden),
                "`{forbidden}` 가 app-server argv 에 실렸다: {argv:?}"
            );
        }
    }

    /// 프로세스 cwd 는 두 모드가 같다 — 워크스페이스 폴더를 어디로 나르든 프로세스는 같은 곳에서 뜬다.
    #[test]
    fn process_cwd_is_the_same_in_both_modes() {
        let cwd = PathBuf::from("C:/workspace");
        assert_eq!(spec(&codex(vec![]), "C:/workspace").cwd, cwd);
        assert_eq!(spec(&codex_app_server(vec![]), "C:/workspace").cwd, cwd);
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
    fn app_server_extra_args_come_last() {
        let s = spec(&codex_app_server(vec!["-c", "model=x"]), "C:/workspace");
        let argv = codex_argv(&s);
        assert_eq!(
            argv,
            vec!["app-server", "--stdio", "-c", "model=x"],
            "하위 명령은 맨 앞이어야 한다 — 뒤로 밀리면 인자로 읽힌다"
        );
    }

    /// 두 모드가 **다른** 통로 모양을 신고하고, 실물도 그 신고를 따른다.
    #[test]
    fn the_two_modes_declare_different_transport_shapes() {
        assert_eq!(
            CodexBackend.transport_shape(&codex(vec![])),
            TransportShape::Pty
        );
        assert_eq!(
            CodexBackend.transport_shape(&codex_app_server(vec![])),
            TransportShape::StdioBidiJson
        );
    }

    /// ★`Raw` 였다면 세션이 본문 뒤에 CR 을 **별도 호출**로 한 번 더 내고, 봉투를 스스로 만드는 통로는
    /// 그것을 또 하나의 본문으로 읽어 빈 턴을 연다★.
    #[test]
    fn app_server_encoder_passes_bytes_through_and_asks_for_no_submit_byte() {
        let enc = CodexBackend.input_encoder(&codex_app_server(vec![]));
        assert_eq!(enc, InputEncoder::TransportFramed);
        assert_eq!(enc.submit_sequence(), None);
        let body = b"hello codex";
        assert_eq!(enc.encode(body, Uuid::new_v4()), body.to_vec());
    }

    /// ★되살리지 마라 — 합성 입력 에코는 기각된 대안이다(ADR-0193 「거부한 대안」)★: codex 가
    /// `userMessage` item 으로 되울리는 것과 겹쳐 같은 질문이 화면에 두 벌 남는다(dedup 키가 서로
    /// 다르다). 유저 발화를 화면에 올리는 것은 그 되울림의 **번역**이 진다(이 폴더 `decoder`).
    /// ★두 모드를 다 재는 것이 요점이다★ — 세션 층은 backend 가 아니라 [`InputEncoder`] 를 들고 dispatch
    /// 표를 지나므로, 선언만 보고 그 표를 안 보면 되살아난 에코가 초록인 채 지나간다.
    #[test]
    fn neither_mode_makes_a_synthetic_input_echo() {
        for command in [codex_app_server(vec![]), codex(vec![])] {
            let enc = CodexBackend.input_encoder(&command);
            assert!(
                enc.input_echo_event("안녕 codex".as_bytes(), Uuid::new_v4())
                    .is_none(),
                "{enc:?}: 합성 에코가 되살아났다"
            );
        }
    }

    #[test]
    fn the_turn_end_event_is_the_ended_signal() {
        let classify = CodexBackend.turn_classifier();
        assert_eq!(
            classify(&OutputEvent::TurnEnd {
                turn_id: Some("u-1".into()),
                outcome: TurnOutcome::Completed
            }),
            Some(TurnSignal::Ended)
        );
        // 결말이 무엇이든 턴은 끝난 것이다 — 실패·중단·미상이 여기서 갈리면 그 결말의 대기 표시가 남는다.
        for outcome in [
            TurnOutcome::Failed {
                detail: Some("boom".into()),
            },
            TurnOutcome::Interrupted,
            TurnOutcome::Unknown,
        ] {
            assert_eq!(
                classify(&OutputEvent::TurnEnd {
                    turn_id: None,
                    outcome: outcome.clone()
                }),
                Some(TurnSignal::Ended),
                "{outcome:?}"
            );
        }
        // 되울린 유저 메시지 — 이 턴이 시작됐다는 첫 관측이다.
        assert_eq!(
            classify(&OutputEvent::Structured {
                kind: "user".into(),
                json: "{}".into()
            }),
            Some(TurnSignal::Progress)
        );
        assert_eq!(
            classify(&OutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: None,
                message_id: None
            }),
            Some(TurnSignal::Progress)
        );
        // `error` 알림은 재시도 가능한 스트림 오류라 턴 경계가 아니다.
        assert_eq!(classify(&OutputEvent::Error("boom".into())), None);
        assert_eq!(
            classify(&OutputEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                turn_id: None
            }),
            None
        );
    }

    /// 터미널 모드는 decoder 가 없어 `TerminalBytes` 만 흐른다 — 그래서 같은 분류자를 모드별 분기 없이
    /// 공유해도 그 모드에서는 신호가 하나도 나오지 않는다.
    #[test]
    fn terminal_bytes_carry_no_turn_signal() {
        assert_eq!(
            CodexBackend.turn_classifier()(&OutputEvent::TerminalBytes(b"[0m".to_vec())),
            None
        );
    }

    #[test]
    fn only_app_server_mode_gets_a_decoder() {
        assert!(CodexBackend.output_decoder(&codex(vec![])).is_none());
        assert!(CodexBackend
            .output_decoder(&codex_app_server(vec![]))
            .is_some());
    }

    /// 이 백엔드가 바뀔 때 **함께** 봐야 하는 짝이라 한 항목에 둔다 — 위 두 impl 바로 옆이고, 켜는
    /// 조건도 하나(그 배선 커밋)다.
    #[test]
    fn neither_session_axis_is_on_in_either_mode() {
        for c in [codex(vec![]), codex_app_server(vec![])] {
            assert!(
                !CodexBackend.assigns_session_id(&c),
                "{c:?}: 발급 주체는 codex 다 — 우리 uuid 를 심으면 그 값은 영영 안 쓰인다"
            );
            assert!(
                !CodexBackend.can_resume_stored_session(&c),
                "{c:?}: 이어받기 배선이 없는 동안은 선언도 false 여야 한다 — 켜는 조건은 그 impl 주석"
            );
        }
    }

    #[test]
    fn reads_messages_is_false() {
        assert!(!CodexBackend.reads_messages());
    }

    #[test]
    fn capabilities_resume_is_false() {
        assert!(!CodexBackend.capabilities(&codex(vec![])).session.resume);
    }

    // ── ADR-0185: 받아 온 thread id 가 조립점의 기록 동사까지 실제로 간다 ──────────────────

    /// 가짜 app-server 스크립트를 지운다 — 지웠으면 `true`. ★한 번 시도하고 마는 모양은 `%TEMP%` 에
    /// 파일을 남겼다★: `shutdown()` 이 돌아온 뒤에도 자식이 그 파일 핸들을 잠깐 더 쥐고 있어 첫
    /// `remove_file` 이 실패한다.
    /// ★결과를 돌려주는 이유 = `eprintln!` 은 **통과한** 항목에서 libtest 가 삼킨다★ — 그 자리에 적으면
    /// 지우지 못한 사실이 아무 데도 안 남고 찌꺼기만 쌓인다. 호출자가 단언으로 올린다.
    #[cfg(windows)]
    #[must_use = "지우지 못한 사실을 버리면 찌꺼기가 조용히 쌓인다 — 단언으로 올릴 것"]
    fn remove_script(path: &std::path::Path) -> bool {
        for _ in 0..40 {
            if std::fs::remove_file(path).is_ok() || !path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// 핸드셰이크의 두 요청에만 답하는 최소 app-server. ★실 codex 가 아니다★ — 재는 것은 「상대가 준
    /// thread id 가 [`AgentBackend::open_spawn`] 에 건넨 동사까지 오나」 하나이고, 그 답에 실 CLI 는
    /// 무관하다(ADR-0012 격리).
    /// ★인자로 넘기지 않고 파일로 굽는다★ — 스크립트에 JSON 이 들어 있어 `"` 가 필수인데, 그 문자는
    /// Rust `Command` 의 인자 이스케이프와 powershell 의 명령줄 해석을 거치며 두 번 씹힌다.
    /// ★stdout 은 raw 바이트로 쓴다★ — `Write-Output` 은 인코딩·BOM 이 호스트 설정에 딸려 가고, BOM 한
    /// 바이트가 첫 줄을 JSON 이 아니게 만든다.
    ///
    /// ★**이 가짜는 답한 뒤에도 죽지 않고 stdin 을 계속 읽는다 — 그 성질이 한 항목의 전제다**★.
    /// [`tests::a_recording_failure_ends_the_session`] 이 재는 것은 「기록이 실패하면 우리가 stdin 을 닫고
    /// 그 결과로 세션이 끝난다」인데, 이 가짜가 스스로 끝나 버리면 EOF 가 **닫기와 무관하게** 와서 그
    /// 항목이 배선을 하나도 안 재고 초록이 된다. 실제로 그렇지 않다는 것은 변이로 확인했다 — 닫기를
    /// 없애면 그 항목에 상태 전이가 **하나도** 관측되지 않는다. ★그러니 아래 while 루프를 「응답 뒤
    /// exit」 으로 바꾸면 그 항목이 조용히 무력해진다★.
    #[cfg(windows)]
    const FAKE_APP_SERVER_PS1: &str = r#"
$ErrorActionPreference = 'Stop'
$so = [Console]::OpenStandardOutput()
function Send([string]$s) {
  $b = [Text.Encoding]::ASCII.GetBytes($s + "`n")
  $so.Write($b, 0, $b.Length)
  $so.Flush()
}
while ($null -ne ($line = [Console]::In.ReadLine())) {
  if ($line -match '"id":(-?\d+)') {
    $rid = $Matches[1]
    if ($line -match '"method":"initialize"') {
      Send ('{"id":' + $rid + ',"result":{"codexHome":"h","platformFamily":"windows","platformOs":"windows","userAgent":"engram-fake/0"}}')
    } elseif ($line -match '"method":"thread/start"') {
      Send ('{"id":' + $rid + ',"result":{"thread":{"id":"THREAD_ID_PLACEHOLDER","cliVersion":"0.0.0-fake"}}}')
    }
  }
}
"#;

    /// ★조립점이 `None` 을 넘기던 시절에는 이 항목이 서지 않았다★ — 배선이 다시 끊기면 여기서 잡힌다.
    /// 재는 것은 **기록 동사가 불렸나**이고, 그 값이 프로필에 어떻게 앉나는 `manager.rs` 쪽 항목이 잰다.
    #[cfg(windows)]
    #[test]
    fn an_app_server_spawn_hands_the_thread_id_to_the_sink() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let thread_id = Uuid::new_v4().to_string();
        let script = FAKE_APP_SERVER_PS1.replace("THREAD_ID_PLACEHOLDER", &thread_id);
        let script_path =
            std::env::temp_dir().join(format!("engram-fake-app-server-{}.ps1", Uuid::new_v4()));
        std::fs::write(&script_path, script).expect("가짜 app-server 기록");

        let spec = CommandSpec {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script_path.to_string_lossy().into_owned(),
            ],
            env: vec![],
            cwd: PathBuf::from("."),
        };

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };

        let parts =
            crate::backend::open_spawn(&codex_app_server(vec![]), &spec, 80, 24, Some(sink))
                .expect("open_spawn");

        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // 핸드셰이크는 powershell 기동 + 두 왕복이라 즉시 끝나지 않는다. 시한은 통로 자신의 요청 시한
        //   (30s)보다 짧게 둔다 — 넘기면 실패 사유가 「우리가 덜 기다렸다」로 흐려진다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let got = seen.lock().unwrap().clone();

        parts.transport.shutdown();

        // ★두 실패를 갈라 적는다★ — 「한 번도 안 불렸다」는 배선이 끊긴 것일 수도, 가짜 app-server 가
        //   시한 안에 안 뜬 것일 수도 있다(powershell 기동·실행 정책). 한 문장으로 적으면 환경 문제를
        //   배선 회귀로 읽는다. ★그래서 이 갈래에서는 스크립트를 지우지 않는다★ — 손으로 돌려 봐야 갈린다.
        assert!(
            !got.is_empty(),
            "시한(20초) 안에 기록 동사가 한 번도 불리지 않았다 — 배선이 끊겼거나, 가짜 app-server 가 그 안에 \
             핸드셰이크를 끝내지 못했다(powershell 기동 실패·실행 정책). 가르려면 {} 를 손으로 돌려 볼 것",
            script_path.display()
        );
        let removed = remove_script(&script_path);
        assert_eq!(
            got,
            vec![thread_id],
            "기록 동사가 app-server 가 준 thread id 와 다른 값을 받았다"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
    }

    /// ★기록이 패닉하면 세션이 **끝난다** — 조용히 멈추지도, 영원히 떠 있지도 않는다★.
    ///
    /// 이 항목이 재는 것은 셋이고 ★셋째가 핵심★이다:
    ///   1. 기록 동사가 실제로 불렸다(= 핸드셰이크가 섰다. 아니면 아래 둘이 공회전한다).
    ///   2. 입력이 거절된다 — 링크가 `Connecting` 에 멈춰 큐만 차던 모양이 아니다(ADR-0190).
    ///   3. ★세션이 **종료 상태에 닿는다**★. 2 만 재던 옛 모양은 **막힌 세션과 구별이 안 됐다** —
    ///      거절은 되는데 자식·리더·라이터가 그대로 남고 pump 가 영영 안 끝나 수거도 안 되는 상태가
    ///      2 를 그대로 통과한다. 그 갈래를 가르는 것은 종료 전이 하나뿐이다.
    #[cfg(windows)]
    #[test]
    fn a_recording_failure_ends_the_session() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputEvent, StatusSink};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        /// 종료 전이를 보는 눈 — `is_live()` 가 거짓인 상태가 곧 종료다(그 술어가 정본).
        struct RecordingStatus(Arc<Mutex<Vec<AgentStatus>>>);
        impl StatusSink for RecordingStatus {
            fn status_changed(&self, _id: Uuid, s: AgentStatus, _e: u32) {
                self.0.lock().expect("status poisoned").push(s);
            }
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let script =
            FAKE_APP_SERVER_PS1.replace("THREAD_ID_PLACEHOLDER", &Uuid::new_v4().to_string());
        let script_path =
            std::env::temp_dir().join(format!("engram-fake-app-server-{}.ps1", Uuid::new_v4()));
        std::fs::write(&script_path, script).expect("가짜 app-server 기록");

        let spec = CommandSpec {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script_path.to_string_lossy().into_owned(),
            ],
            env: vec![],
            cwd: PathBuf::from("."),
        };

        // 불렸다는 사실만 남기고 터진다 — 「패닉이 났다」와 「아예 안 불렸다」를 갈라야 하기 때문.
        let reached = Arc::new(AtomicBool::new(false));
        let sink: SessionIdSink = {
            let reached = reached.clone();
            Arc::new(move |_: &str| {
                reached.store(true, Ordering::SeqCst);
                panic!("기록 포트가 터졌다");
            })
        };

        let parts =
            crate::backend::open_spawn(&codex_app_server(vec![]), &spec, 80, 24, Some(sink))
                .expect("open_spawn");

        let statuses: Arc<Mutex<Vec<AgentStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let terminal = {
            let statuses = statuses.clone();
            move || {
                statuses
                    .lock()
                    .expect("status poisoned")
                    .iter()
                    .any(|s| !s.is_live())
            }
        };

        // ★훅 교체를 맨손으로 하지 않는다★ — 전역이라 같은 바이너리의 다른 항목이 자기 패닉 출력을 잃고,
        //   중첩되면 조용한 훅이 영구히 남는다. 그 둘을 막는 헬퍼를 쓴다. 라이터 스레드가 이 구간 안에서
        //   터지므로 구간이 그 시점을 덮어야 한다.
        // ★조용한 훅 구간은 **패닉 순간까지만** 잡는다★ — 그 훅은 프로세스 전역이고 헬퍼가 static
        //   뮤텍스로 직렬화하므로, 종료 대기까지 감싸면 같은 바이너리의 다른 패닉 항목이 그만큼 줄을 서고
        //   그 창에 터진 **무관한** 항목이 자기 패닉 메시지와 위치를 잃는다. 종료 대기는 패닉과 무관하므로
        //   구간 밖으로 뺀다(최악 대기가 절반으로 준다).
        let (reached, refusal) = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            parts.transport.start(Arc::new(OutputCore::new(
                Uuid::new_v4(),
                1,
                Arc::new(RecordingStatus(statuses.clone())),
                TurnWiring::detached(),
            )));

            // ★먼저 기록 동사가 불릴 때까지 **아무것도 보내지 않고** 기다린다★ — 핸드셰이크 전의
            //   `send_input` 은 정상적으로 큐에 서므로(ADR-0190), 여기서 보내면 상한(32)을 채워
            //   **큐 가득참 오류**가 나고 이 항목이 엉뚱한 이유로 초록이 된다.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !reached.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            // 링크를 내리는 것은 그 다음 몇 마이크로초다. 시도는 상한보다 한참 적게 둔다 — 같은 이유.
            let mut refusal = None;
            for _ in 0..8 {
                match parts.transport.send_input(InputEvent::Raw(b"hi".to_vec())) {
                    Err(e) => {
                        refusal = Some(e);
                        break;
                    }
                    Ok(()) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }

            (reached.load(Ordering::SeqCst), refusal)
        });

        // stdin 을 놓은 뒤 상대가 스스로 끝나고(실측 41–51ms) 그 EOF 가 pump 를 끝내기까지 기다린다.
        //   ★`shutdown()` 을 부르지 않고 기다리는 것이 요점이다★ — 우리가 죽여 놓고 「끝났다」를 재면 이
        //   항목이 재려던 그 인과를 우리가 대신 굴린 것이 된다.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !terminal() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let reached_terminal = terminal();
        let seen = statuses.lock().expect("status poisoned").clone();
        // ★`!is_live()` 로는 부족하다★ — 리더 패닉은 `Failed`, 읽기 오류는 `Exited{code:None}` 을 내는데
        //   **자식이 살아 있어도** 둘 다 그 술어를 만족한다. 이 항목이 재려는 것은 「상대가 EOF 를 보고
        //   스스로 끝났다」이므로 종류까지 못 박는다. 오늘 이 시험대에 그 둘을 만드는 경로가 없지만, 못
        //   박아 두지 않으면 나중에 그리로 흘러가도 초록이다.
        let ended_by_peer_exit = seen.iter().any(|s| matches!(s, AgentStatus::Exited { .. }));
        parts.transport.shutdown();

        assert!(
            reached,
            "기록 동사가 한 번도 불리지 않았다 — 이 항목은 실패 처리를 재지 못했다. 가짜 app-server 가 안 \
             떴을 수 있다(스크립트는 남겨 둔다): {}",
            script_path.display()
        );
        let removed = remove_script(&script_path);

        let refusal = refusal.expect(
            "기록이 실패했는데 입력이 계속 받아들여진다 — 링크가 `Connecting` 에 멈춰 세션이 조용히 벙어리가 됐다",
        );
        let PtyError::WriteFailed(reason) = &refusal else {
            panic!("예상 밖 오류 종류: {refusal:?}");
        };
        // ★큐 가득참만 배제한다★ — 그것만이 「실패 경로가 안 돌았는데 거절됐다」를 뜻한다. 남은 두 사유
        //   (기록 실패로 내려간 링크 · 그 뒤 stdin 을 닫아 끝난 통로)는 **둘 다 이 경로가 돈 증거**이고,
        //   어느 쪽이 잡히나는 타이밍이라 하나로 못 박으면 그 자체가 깜빡이가 된다.
        assert!(
            !reason.contains("상한"),
            "거절 사유가 큐 가득참이다 — 실패 경로가 돈 것을 잰 것이 아니다: {reason}"
        );

        assert!(
            reached_terminal,
            "기록이 실패했는데 세션이 종료 상태에 닿지 않았다 — 자식·리더·라이터가 그대로 붙들려 있고 pump \
             도 reaper 도 움직이지 않는다(관측된 상태: {seen:?})"
        );
        assert!(
            ended_by_peer_exit,
            "종료는 했는데 상대가 스스로 끝난 모양이 아니다 — `Failed` 와 `Exited{{code:None}}` 는 자식이 살아 있어도 나므로 이 항목이 재려는 인과(stdin 닫기 → 상대 exit → EOF)를 재지 못한다(관측: {seen:?})"
        );
        assert!(
            removed,
            "가짜 app-server 스크립트를 지우지 못했다: {}",
            script_path.display()
        );
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
