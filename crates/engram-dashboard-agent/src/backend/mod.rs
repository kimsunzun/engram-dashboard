//! AgentBackend — 백엔드별 명령 명세 산출 trait + 자유 함수 dispatch.
//!
//! transport(PtyTransport)는 claude/codex를 모른다. 누가 어떤 프로그램인지 아는 곳은
//! 오직 `backend/<이름>/` 폴더다.
//!
//! ★이 파일은 어느 백엔드의 항목도 이름으로 부르지 않는다★: 백엔드 이름이 적히는 자리는 **등록부**
//! (`pub mod`·`pub use`·정적 싱글턴)와 **두 dispatch 표**(`backend_for` · `backend_for_encoder`)뿐이고,
//! 백엔드별 지식은 전부 [`AgentBackend`] 메서드로만 나온다. 게이트는 `backend/claude/mod.rs` 헤더.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use shell::ShellBackend;

use std::path::PathBuf;

use uuid::Uuid;

use crate::failure::AgentFailureKind;
use crate::profile::{AgentCommand, SpawnMode};
use crate::session_tracker::SessionIdSource;
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, OutputDecoder};
use crate::turn::TurnSignal;
use crate::types::{AgentId, BackendCaps, CommandSpec, ControlEndpoint, OutputEvent, PtyError};

/// **왜 필요한가:** Windows에서 `claude`는 확장자 없는 npm shim이라, ConPTY가 쓰는 CreateProcessW가
/// 직접 못 띄운다(error 193 — PATHEXT/셸 해석을 안 함). `cmd.exe /c <prog> …`로 감싸면 cmd가
/// `<prog>.cmd` shim을 해석해 실제 프로세스를 띄운다. `cmd /c`는 대상이 종료되면 함께 종료되므로
/// "PTY 자식 = 에이전트" 수명이 유지된다(JobObject가 트리 통째 kill). 비Windows는 그대로 직접 실행.
///
/// shim이 아닌 일반 실행파일(Shell의 cmd.exe 등)에는 적용하지 않는다 — CLI 백엔드 전용.
pub(crate) fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
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

/// unit struct로 구현되어 &'static으로 사용된다 — 상태 없음.
pub trait AgentBackend: Send + Sync {
    /// true면 manager가 sid를 발급·watcher를 부착한다.
    fn needs_session(&self) -> bool;

    /// 이 백엔드가 데몬 제어 채널(MCP 입구)을 **소비**하는가(ADR-0086 F3).
    /// true 면 manager 가 spawn 전에 provision 을 부르고(토큰+mcp-config 발급), 그 endpoint 를
    /// build_spec 에 넘긴다(claude=`--mcp-config`). false 면 manager 가 provision 을 **아예 건드리지
    /// 않는다** — shell 처럼 제어 채널을 안 쓰는 backend 는 registry 에 손대지 않아, config-write 실패가
    /// MCP 가 필요 없던 스폰을 중단시키는 회귀(round-2 F3)가 생기지 않는다.
    ///
    /// ★fail-closed 는 provision 을 **부르는** backend 에만★: true 인 backend 는 provision 이 Err 면
    ///   스폰이 중단된다(제어 채널 없이 몰래 도는 에이전트 금지). false 인 backend 는 그 계약과 무관하다.
    fn supports_control_channel(&self) -> bool;

    /// 이 backend(프로그램)가 **MCP config 를 받아들일 수 있는가**(ADR-0099). claude=true(mcp-config 파일을
    /// `--mcp-config` 로 붙임), 그 외(shell·codex·gemini stub)=false. ★backend 지식(ADR-0004)★: "어느
    /// 프로그램이 MCP config 를 소비하나"는 backend-kind 지식이라 여기서 선언한다 — manager 가 `matches!`
    /// 로 직접 분기하지 않는다.
    ///
    /// 이 플래그 하나가 provision 의 MCP 입구·grant·프라이밍 변형·**우편 가부**를 전부 구동한다(정합
    /// 불변식 = 프라이밍이 가르치는 우편 채널 **=** 그 스폰이 쓸 수 있는 우편 채널. 못 쓰는 채널을 가르치면
    /// 발신 freeze 가 재발하고, 쓸 수 있는데 안 가르치면 통제 없는 우회 표면이 남는다). true 면
    /// `DaemonControlChannel::provision` 이 mcp-config 를 쓰고 MCP bits 를 endpoint 에 실으며 MCP-only 교육
    /// 프라이밍(`send_message` 만 — ADR-0126 결정 1)과 우편 불가 표식을, false 면 mcp-config 미기록 +
    /// CLI-only 프라이밍 + 우편 가능 표식을 고른다(ADR-0133). 제어 CLI 배선은 이 축과 무관하게 전원에게 간다.
    ///
    /// ★`supports_control_channel` 과의 관계★: 후자는 "provision 을 **부르나**"(제어 채널 자체를 소비하나),
    ///   이것은 "provision 이 붙일 채널 중 **MCP 를 낄 수 있나**"다 — 직교 축이다. 현재 claude 는 둘 다 true,
    ///   codex/gemini 는 둘 다 false 지만, 미래 "제어 채널은 CLI 로만 쓰는 백엔드"는 전자 true·후자 false 다.
    // ADR-0126
    // ADR-0133
    fn accepts_mcp_config(&self) -> bool;

    /// cwd·env는 manager가 정규화한 값을 전달한다.
    ///
    /// `control`(ADR-0086): 데몬이 발급한 제어 채널 엔드포인트(추상 descriptor). 있으면 backend 가
    ///   자기 프로그램 방식으로 명령줄에 주입한다(claude=`--mcp-config <path>` — 그 지식은
    ///   `backend/claude/` 단독, ADR-0004). None 이거나 제어 채널을 안 쓰는 backend(shell)면 무시한다.
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec;

    /// 이 backend(프로그램)가 결정하는 caps — session(resume)·model.
    /// transport(물리 채널)가 만드는 input/output/control 과 별개로, 최종 Capabilities 는
    /// `Capabilities::compose(transport_caps, backend_caps)` 로 합성된다.
    ///
    /// `command` 를 받는 이유(FIX 5): 같은 프로그램(claude)이라도 **모드에 따라 caps 가 다르다** —
    /// json(stream-json) 모드는 resume 미지원(ADR-0044 후속)이라 resume=false 를 신고해야 한다.
    /// backend 가 session caps 의 출처(ADR-0030)이고 mode 는 command 에 있으므로, 여기서 command 를
    /// 보고 정직하게 산출한다(type split 유지 — output/control 은 여전히 transport 소관).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps;

    /// 이 backend 가 `command` 에 대해 요구하는 **물리 통로 모양**.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 프로그램을 어떤 통로로 띄워야 하나" 는 프로그램별 지식이다.
    /// ★공용 층에 판정 술어를 되살리지 말 것★: 조립점이 `AgentCommand` payload 를 직접 되묻는 순간 한
    ///   backend 의 출력 형식 축이 전원의 통로 선택을 굴리게 되고, 그 backend 를 안 쓰는 항목까지 그
    ///   축을 통과한다.
    /// ★렌더 모드 축과 같은 것이 아니다★: 그 프로그램이 무엇을 그리나(터미널 TUI ↔ 구조화 스트림)는
    ///   그 backend 의 명령 payload 가 갖는 축이고, 이것은 그 선택이 **우리 쪽 통로**에 무엇을 요구하나다.
    ///   한 backend 안에서 둘이 1:1 로 붙어 있어도 어휘를 합치지 말 것 — 합치면 새 backend 가 자기 렌더
    ///   모드를 우리 통로 이름으로 신고해야 한다.
    /// ★기본값 = `Pty`★: 선언하지 않은 backend 는 터미널로 뜬다.
    // ADR-0004
    // ADR-0044
    fn transport_shape(&self, _command: &AgentCommand) -> TransportShape {
        TransportShape::Pty
    }

    /// 이 backend 가 `command` 를 띄울 **물리 통로를 직접 만들어**, 세션 조립에 필요한 나머지 값과 함께
    /// 한 번에 내준다.
    ///
    /// ★왜 backend 인가(ADR-0191)★: "이 프로그램을 어떤 통로 구현체로 띄우나" 는 프로그램별 지식이다.
    ///   조립점이 [`TransportShape`] 를 다시 match 해 생성자를 고르면 **가르는 자리가 두 곳**이 되고
    ///   (`backend_for` + 그 match), 그 자리가 백엔드 전용 통로 타입을 이름으로 알게 된다(ADR-0004).
    /// ★조립점은 돌려받은 통로의 실제 타입을 모른다★ — `Box<dyn AgentTransport>` 로만 받는다.
    /// ★아스펙트를 한 dispatch 에 모아 싣는다★: caps·인코더·턴 분류자·우편 자격은 각자 자기 메서드가
    ///   그대로 소유하고, 여기서는 그것들을 **모으기만** 한다. 조립점이 같은 switch 를 아스펙트마다 다시
    ///   타지 않게 하는 것이 이 묶음의 존재 이유다.
    /// ★`output.structured` 를 통로 구현체가 하드코딩하는 것은 여전히 금지(ADR-0044/0030)★ — 구조화
    ///   파이프를 고른 backend 가 그 자리에서 주입한다. 규칙은 그대로이고 주입하는 **자리**만 여기다.
    /// `cols`/`rows` 는 터미널 통로에만 쓰인다 — 파이프에는 크기 개념이 없어 무시된다.
    ///
    /// ★기본값 = PTY + 각 아스펙트가 신고한 값★: 통로를 따로 만들지 않는 backend 는 터미널로 뜨고
    ///   ([`AgentBackend::transport_shape`] 기본값과 같은 자리), 나머지 칸은 자기 메서드의 산출을 그대로
    ///   싣는다.
    /// ★단 이 기본값은 `transport_shape` 를 **읽지 않는다**★: 파이프를 요구한다고 신고해 놓고 이 메서드를
    ///   구현하지 않으면 조용히 터미널로 뜬다. 선언 표 트립와이어(`tests::expected_codec_axis`)는
    ///   `transport_shape` 의 신고값만 재므로 그 어긋남을 못 본다.
    // ADR-0004
    // ADR-0191
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<SpawnParts, PtyError> {
        let (transport, child_pid) = PtyTransport::open(spec, cols, rows)?;
        Ok(SpawnParts {
            transport: Box::new(transport),
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
        })
    }

    /// 이 backend 의 **턴 신호 분류자**(ADR-0113 사실 계층의 백엔드 지식 몫).
    ///
    /// ★왜 backend 인가(ADR-0004 · ADR-0110 결정 4 의 취지 승계)★: "어떤 출력 이벤트가 턴 진행이고
    ///   어떤 게 턴 종료인가" 는 프로그램별 지식이다. 특히 `OutputEvent::Structured` 는 **백엔드별
    ///   이벤트 탈출구**라 그 `kind` 의 의미가 백엔드마다 다르다 — 공용 층에서 해석하면 한 백엔드의
    ///   관례가 전원에게 강제된다(구조화 메타 라인을 내는 백엔드가 종료 신호 없이 영구 "턴 중" 이 된다).
    /// ★신호 어휘(`TurnSignal`)는 공용 하나뿐★: 백엔드마다 다른 건 **매핑**이지 어휘가 아니다.
    ///   백엔드별 신호 enum 을 만들면 소비자가 백엔드를 알아야 한다.
    /// ★기본값 = 신호 없음(fail-open)★: 매핑을 선언하지 않은 backend 는 관측 대상이 아니다. 미관측은
    ///   소비자 쪽에서 "즉시 배달" 로 흡수되지만(positive-knowledge-only), 근거 없는 진행 신호는
    ///   깨울 수 없는 "턴 중" 을 만든다 — 그래서 기본은 침묵이다.
    // ADR-0004
    // ADR-0113
    fn turn_classifier(&self) -> TurnClassifier {
        no_turn_signals
    }

    /// 이어받기가 실패했을 때 **그 프로그램의 출력에서 원인을 알아볼 수 있나**(ADR-0172 분류 입구).
    ///
    /// `evidence` = 그 화신이 남긴 텍스트(best-effort — 비어 있을 수 있다). 호출자가 **두 스트림을
    /// 합쳐** 넘긴다: 콘솔 꼬리(PTY 세션) + 진단 stderr 꼬리(파이프 세션). 둘은 transport 에 따라
    /// 배타적으로 차므로 구현체는 어느 쪽에서 왔는지 알 필요가 없다 — 문구만 본다.
    /// `None` = 이 텍스트만으로는 종류를 단정할 수 없다 → 호출자가 맥락 기본값으로 떨어뜨린다.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 문구가 무슨 뜻인가" 는 프로그램별 지식이다. manager 가 문자열을
    ///   직접 보면 그 지식이 공용 층으로 샌다.
    /// ★기본값 = 모름(fail-open)★: 선언하지 않은 backend 는 아무 것도 단정하지 않는다.
    /// ★살아 있는 세션에도 불린다★: 호출자는 종료를 기다리지 않고 진단 스트림만으로 확정할 수 있다
    ///   (`EarlyVerdict::Diagnosed`). 그러니 **"그 프로그램이 실패했을 때만 낼 수 있는 문구"** 만
    ///   선언할 것 — 대화 본문에 섞여 나올 수 있는 문구를 선언하면 성공한 활성화가 실패로 도장 찍힌다.
    // ADR-0004
    // ADR-0172
    fn resume_failure_kind(&self, _evidence: &str) -> Option<AgentFailureKind> {
        None
    }

    /// 이 backend 가 **편지를 읽는 주체**인가 = 에이전트 간 우편의 수신자 명단 자격.
    ///
    /// ★"턴 관측 가능성" 축이 아니다(ADR-0116 결정 1·7 을 되돌리는 게 아니다)★: 구조화 출력이 없는
    ///   터미널 claude 는 **그대로 받는다**. 여기서 가르는 건 관측 가능성이 아니라 **입력이 무엇으로
    ///   해석되는가** 다 — shell 은 입력을 명령으로 **실행**하는 채널이라 봉투가 닿으면 읽히는 게 아니라
    ///   실행된다(본문은 LLM 자유 텍스트라 `&`·`|`·`;` 가 섞이면 그 뒤가 별도 명령으로 파싱된다).
    /// ★기본값 = true(fail-open)★: 새 CLI 백엔드는 편지를 읽는 쪽이 정상이다. 모른다고 배달을 끊으면
    ///   편지가 조용히 사라지므로, 실행 채널인 backend 만 스스로 false 를 선언한다.
    // ADR-0004
    fn reads_messages(&self) -> bool {
        true
    }

    /// 이 backend 가 `command` 에 대해 쓰는 **입력 인코딩 태그**. 세션이 spawn 때 한 번 받아 보관한다.
    ///
    /// ★왜 backend 인가(ADR-0004)★: "이 프로그램이 stdin 을 무엇으로 읽나" 는 프로그램별 지식이다.
    ///   dispatch 층이 명령 모양을 직접 보고 태그를 고르면 그 지식이 공용 층으로 샌다.
    /// ★기본값 = `Raw`★: 모르는 프로그램의 stdin 에 지어낸 봉투를 씌우면 그 프로그램은 입력을 통째로
    ///   못 읽는다 — 그래서 선언하지 않은 backend 는 바이트를 그대로 통과시킨다.
    // ADR-0004
    // ADR-0044
    fn input_encoder(&self, _command: &AgentCommand) -> InputEncoder {
        InputEncoder::Raw
    }

    /// 텍스트 1턴을 이 backend 의 구조화 입력 라인으로 감싼다 — [`InputEncoder::encode`] 의 실물.
    ///
    /// `msg_uuid` 는 [`AgentBackend::input_echo_event`] 에 넘어가는 값과 **반드시 같다**(호출자가 한 번
    /// 생성해 양쪽에 넘긴다 — dedup 키).
    /// ★기본값 = 감싸지 않음★: `input_encoder` 가 `Raw` 인 backend 는 [`backend_for_encoder`] 가 `None` 을
    ///   줘 이 경로를 아예 타지 않는다. 계약을 총(total)으로 두려는 자리채움이고, 불려도 `Raw` 와 같은
    ///   바이트를 낸다.
    fn wrap_input_turn(&self, text: &str, _msg_uuid: Uuid) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// 입력 성공 직후 세션 층이 emit 할 입력-시점 유저 에코 이벤트. `None` = 만들지 않는다.
    ///
    /// ★이벤트 shape 는 그 backend 의 decoder 가 replay 에 대해 만드는 것과 같아야 한다★ — 프론트가
    ///   uuid 로 둘을 합치므로 어긋나면 화면에 두 개로 남는다. 그 shape 는 구현체 소유다.
    /// ★기본값 = 없음★: PTY 가 입력을 즉시 로컬 에코하는 채널은 합성 에코를 만들면 중복이 된다.
    // ADR-0044
    // ADR-0045
    fn input_echo_event(&self, _text: &str, _msg_uuid: Uuid) -> Option<OutputEvent> {
        None
    }

    /// pump→core 앞에 꽂히는 출력 정제 decoder. `None` = 바이트 직통.
    ///
    /// [`AgentBackend::input_encoder`] 의 출력 방향 짝 — 감싼 쪽이 푸는 쪽도 소유한다.
    /// ★기본값 = 없음★: 선언하지 않은 backend 의 출력은 정제 없이 그대로 흐른다(터미널·평문 불변).
    // ADR-0004
    // ADR-0044
    fn output_decoder(&self, _command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        None
    }

    /// resume 스폰 시 이 명령의 과거 대화를 복원한 이벤트 목록. 빈 Vec = seed 안 함.
    ///
    /// transcript 의 파일 배치·포맷 지식은 전부 구현체 몫이다 — manager 는 이 호출만 하고 경로 규칙을
    /// 모른다.
    /// ★기본값 = 빈 Vec★: 선언하지 않은 backend 는 과거를 복원하지 않는다(fresh 버퍼 동작 불변).
    // ADR-0004
    // ADR-0079
    fn resume_transcript_events(
        &self,
        _command: &AgentCommand,
        _cwd: &std::path::Path,
        _session_id: Uuid,
    ) -> Vec<OutputEvent> {
        Vec::new()
    }

    /// 이 화신의 **세션 id 가 바뀐 것을 밖에서 알아볼 수 있나** — 알아본다면 그 관측기.
    ///
    /// 우리가 spawn 때 지정한 sid 는 그 프로그램 안에서 바뀔 수 있다(claude 는 `/clear`·`/resume` 로
    /// 프로세스는 그대로인 채 갈아탄다 — spike 실측). 바뀐 값을 못 따라잡으면 다음 복원이 옛 세션을
    /// 살린다. 그것을 어디서 어떻게 읽나(파일 경로·JSON 스키마·PID 우회)는 전부 구현체 몫이고,
    /// 러너([`crate::session_tracker`])는 주기·명부·콜백만 갖는다.
    ///
    /// `agent_id` 는 구현체 진단 로그의 상관 키로만 쓴다 — 관측 대상을 고르는 데 쓰지 않는다.
    /// ★기본값 = `None`(관측 없음)★: 선언하지 않은 backend 는 추적 대상이 아니다. 근거 없는 관측기를
    ///   기본으로 두면 남의 파일을 읽고 엉뚱한 sid 를 프로필에 적는다.
    /// ★`None` 은 실패가 아니다★: 관측이 없어도 복원은 최초 지정 sid 로 정상 동작한다(best-effort).
    // ADR-0004
    fn session_id_source(
        &self,
        _agent_id: AgentId,
        _child_pid: u32,
        _expected_sid: Uuid,
    ) -> Option<Box<dyn SessionIdSource>> {
        None
    }
}

/// spawn 한 번을 조립하는 데 필요한, backend 가 **한 dispatch 로** 내주는 묶음([`AgentBackend::open_spawn`]).
///
/// ★caps 소유권 분할은 그대로다(ADR-0030)★: `backend_caps` 는 backend 몫뿐이고, 통로가 신고하는
///   input/output/control caps 와는 세션 층에서 `Capabilities::compose` 로 합성된다. 여기서 미리 합치지
///   않는다 — 합치면 두 출처가 한 값으로 뭉개져 어느 쪽이 무엇을 신고했는지 추적이 끊긴다.
// ADR-0191
pub struct SpawnParts {
    pub transport: Box<dyn AgentTransport>,
    pub child_pid: Option<u32>,
    pub backend_caps: BackendCaps,
    pub encoder: InputEncoder,
    pub turn_classifier: TurnClassifier,
    pub reads_messages: bool,
}

/// 출력 이벤트 → 턴 신호 매핑 함수(ADR-0113). 백엔드가 자기 함수를 내주고 `OutputCore` 가 그 포인터를
/// 세션 수명 동안 들고 이벤트마다 부른다.
///
/// ★왜 함수 포인터인가★: emit 은 에이전트 출력마다 도는 hot path 다 — 매 이벤트에 `AgentCommand` 를
///   들고 dispatch 하면 세션이 자기 명령 사본을 들거나 매니저를 되짚어야 하고, `Box<dyn Fn>` 은 할당을
///   낳는다. backend 는 상태 없는 unit struct 라 매핑이 순수 함수로 떨어지므로, spawn 때 한 번 뽑아
///   포인터로 들고 있으면 할당·락·조회가 전부 0 이다.
pub type TurnClassifier = fn(&OutputEvent) -> Option<TurnSignal>;

/// 관측이 필요 없는 조립(테스트 하네스)도 이걸 꽂아 "신호 없음" 을 명시한다.
pub fn no_turn_signals(_event: &OutputEvent) -> Option<TurnSignal> {
    None
}

// ── 정적 싱글턴 ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;
static CODEX_BACKEND: CodexBackend = CodexBackend;

// 새 variant 연결 시: tests::expected_channel_matrix(tripwire)가 의식적 capability 선언을 강제한다 — ADR-0099
fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
        AgentCommand::Codex { .. } => &CODEX_BACKEND,
    }
}

/// 인코딩 태그를 **소유한 backend**. `backend_for`(명령 축)의 인코더 축 짝이다 — 세션 층은 태그만 들고
/// 명령은 갖고 있지 않아(소유권 분할) 여기서 되짚는다.
///
/// `None` = 그 태그에는 backend 지식이 없다(바이트 통과).
/// ★백엔드 이름은 이 표와 바로 위 `backend_for`, 그리고 싱글턴 선언에만 적는다★ — 그 바깥에서 백엔드
///   이름이 나오면 `backend/<이름>/` 폴더 격리가 샌 것이다(ADR-0004).
fn backend_for_encoder(e: InputEncoder) -> Option<&'static dyn AgentBackend> {
    match e {
        InputEncoder::Raw => None,
        InputEncoder::ClaudeStreamJson => Some(&CLAUDE_BACKEND),
    }
}

// ── 자유 함수 dispatch ─────────────────────────────────────────────────────────

pub fn needs_session(c: &AgentCommand) -> bool {
    backend_for(c).needs_session()
}

pub fn supports_control_channel(c: &AgentCommand) -> bool {
    backend_for(c).supports_control_channel()
}

pub fn accepts_mcp_config(c: &AgentCommand) -> bool {
    backend_for(c).accepts_mcp_config()
}

pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    control: Option<ControlEndpoint>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, cwd, env, control)
}

pub fn backend_caps(c: &AgentCommand) -> BackendCaps {
    backend_for(c).capabilities(c)
}

pub fn transport_shape(c: &AgentCommand) -> TransportShape {
    backend_for(c).transport_shape(c)
}

/// 통로 실물도 그 위에 실리는 아스펙트 값도 전부 [`AgentBackend::open_spawn`] 이 소유하고 이 함수는
/// dispatch 뿐이다 — 새 backend 는 자기 폴더에서 그 메서드를 구현하면 되고 이 함수는 손대지 않는다
/// (교체성). ★이 호출이 자식 프로세스를 띄운다★ — 위아래 dispatch 들과 달리 부작용이 있고, 실패하면
/// 그 spawn 이 성립하지 않는다.
// ADR-0191
pub fn open_spawn(
    c: &AgentCommand,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
) -> Result<SpawnParts, PtyError> {
    backend_for(c).open_spawn(c, spec, cols, rows)
}

pub fn turn_classifier(c: &AgentCommand) -> TurnClassifier {
    backend_for(c).turn_classifier()
}

pub fn resume_failure_kind(c: &AgentCommand, evidence: &str) -> Option<AgentFailureKind> {
    backend_for(c).resume_failure_kind(evidence)
}

pub fn reads_messages(c: &AgentCommand) -> bool {
    backend_for(c).reads_messages()
}

pub fn session_id_source(
    c: &AgentCommand,
    agent_id: AgentId,
    child_pid: u32,
    expected_sid: Uuid,
) -> Option<Box<dyn SessionIdSource>> {
    backend_for(c).session_id_source(agent_id, child_pid, expected_sid)
}

// ── 입력 인코딩(ADR-0044/0004) ────────────────────────────────────────────────

/// 세션 입력(write_input)을 transport 로 보내기 **직전** 인코딩 방식. AgentSession 이 spawn 시
/// 받아 보관하고 write_input 마다 적용한다.
///
/// ★설계 의도★: transport 는 항상 raw 바이트만 쓴다(바보 파이프 — ADR-0044). "텍스트 턴을
/// claude JSON 라인으로 감싸는" 지식은 backend 소유다. session 은 이 enum(태그)만 들고, 실제
/// 스키마는 [`AgentBackend::wrap_input_turn`] 구현체 안에만 산다(ADR-0004 격리 — 이 모듈도 session 도
/// transport 도 형태를 모른다).
/// backend 가 요구하는 물리 통로 모양 — **신고값**이다. 통로 실물을 만드는 것은
/// [`AgentBackend::open_spawn`] 이고, 그 안에서도 이 값을 다시 match 해 생성자를 고르지 않는다
/// (ADR-0191 — 가르는 switch 는 `backend_for` 하나뿐). 오늘 이 값을 읽는 곳은 선언 표 트립와이어
/// (`tests::expected_codec_axis`)뿐이다.
///
/// ★출력 구조화 여부가 여기 함의돼 있다★: `StdioNdjson` 은 그 파이프가 나르는 바이트가 줄단위 JSON
/// 이라는 뜻이고, 통로 자신은 그것을 모른다(바보 파이프 — ADR-0044). 그래서 그 `structured` output caps
/// 는 하드코딩되지 않고 주입되는데, 주입하는 쪽이 파이프를 고른 backend 자신이다(ADR-0030 분담 유지).
// ADR-0004
// ADR-0044
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportShape {
    /// 터미널(ConPTY/pty) — 바이트 그대로, resize 있음.
    Pty,
    /// stdio 파이프 + 줄단위 JSON 출력 — resize 개념 없음.
    StdioNdjson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEncoder {
    /// 바이트 그대로 통과(PTY/터미널·shell). 기존 동작과 **바이트 동일**.
    Raw,
    /// claude stream-json: 텍스트 1턴을 user JSON 라인(`\n` 종단)으로 감싼다(스키마 = `backend/claude/`).
    ClaudeStreamJson,
}

impl InputEncoder {
    /// `msg_uuid`: 이 유저 턴의 메시지 uuid(replay dedup 키). ClaudeStreamJson 은 stdin user 라인에
    ///   심어 claude 가 replay 시 그대로 되울리게 한다(uuid dedup 계약 = `backend/claude/`).
    ///   같은 write_input 이 이 uuid 를 input_echo_event 에도 넘겨 합성 에코와 replay 를 uuid 로 합친다.
    ///   Raw(터미널·shell)는 uuid 를 쓰지 않는다(무시) — 바이트 동일 보장 유지.
    pub fn encode(&self, bytes: &[u8], msg_uuid: Uuid) -> Vec<u8> {
        match backend_for_encoder(*self) {
            None => bytes.to_vec(),
            // ※from_utf8_lossy(FIX 6b): 비-UTF8 입력은 U+FFFD 로 치환돼 손상될 수 있으나, 구조화 입력은
            //   텍스트 챗 메시지라 UTF-8 이 전제다(MVP=텍스트 챗, ADR-0044) → 허용.
            Some(b) => b.wrap_input_turn(&String::from_utf8_lossy(bytes), msg_uuid),
        }
    }

    /// 입력 성공 직후 세션 층이 core.emit 할 **입력-시점 유저 에코 이벤트**를 만든다(ADR-0044/0045).
    ///
    /// ★왜 여기(backend) 인가★: 터미널(Raw)은 PTY 가 입력을 즉시 로컬 에코하지만, json 모드는 claude
    ///   가 되울릴 때까지 화면에 안 뜬다. 그 왕복 지연을 없애려 write_input 직후 합성 유저 이벤트를
    ///   emit 한다. 어떤 encoder 가 이 에코가 필요한지·이벤트의 json 스키마가 뭔지는 backend 지식이라
    ///   session 이 아니라 여기서 판정한다(ADR-0004 — session 은 encoder 태그만 들고 형태를 모른다).
    ///   Raw(터미널)는 None 을 돌려줘 세션이 아무 것도 emit 하지 않는다(PTY 가 이미 에코 — 중복 방지).
    ///
    /// ★decoder uuid dedup 과 짝(blunt-suppress → uuid dedup 교체)★: 이 이벤트는 decoder 가 replay 된
    ///   user-role 블록에 대해 만드는 것과 동일 shape(`Structured{kind:"user", json:{"type":"text",
    ///   "text":<raw>,"uuid":"X"}}`)이다. `msg_uuid` 가 stdin(encode)에 심은 값과 같아, 이후 claude 가
    ///   되울린 replay(같은 uuid)를 프론트 accumulator 가 uuid 로 dedup 해 한 개로 합친다. 예전엔 decoder 가
    ///   user text 블록을 blunt 억제해 이 합성 에코가 "자리 대체"했으나, resume 시 과거 대화가 사라지는
    ///   버그라 uuid dedup 으로 바꿨다(과거/비매칭 uuid user text 는 전부 보존).
    pub fn input_echo_event(
        &self,
        bytes: &[u8],
        msg_uuid: Uuid,
    ) -> Option<crate::types::OutputEvent> {
        backend_for_encoder(*self)?.input_echo_event(&String::from_utf8_lossy(bytes), msg_uuid)
    }

    /// 인코딩된 본문 뒤에 **별도 write** 로 한 번 더 내보내야 하는 제출(턴 시작) 바이트.
    /// `None` = 본문 write 하나가 이미 제출이다.
    ///
    /// ★왜 `encode` 안에서 붙일 수 없나(실측 2026-08-17 — 되살리지 마라)★: 살아 있는 claude TUI 세션에
    ///   `본문 + CR` 을 **한 번의 write** 로 넣으면 텍스트가 입력창에 남고 턴이 시작되지 않는다. 제출을
    ///   만드는 건 바이트열이 아니라 **수신자가 그것을 언제 읽느냐**라, 바이트열 하나를 돌려주는 `encode`
    ///   로는 표현할 수 없다 — 그래서 별도 질문이다.
    /// ★write 를 나누는 것만으로는 부족하다★: 나눠 써도 **간격이 없으면** 두 write 가 PTY 에서 한 덩이로
    ///   묶여 수신자가 한 번의 read 로 받고, 결국 합쳐 쓴 것과 같아진다. 그래서 짝이 되는 대기가 필요하고
    ///   그 값이 [`SUBMIT_PACING`] 이다(둘은 항상 같이 간다 — 한쪽만 보고 고치지 말 것).
    /// ★`Raw` = CR★: 터미널·shell 은 PTY 로 사람 키보드를 흉내내는 채널이라 Enter = CR(0x0D) 이다.
    /// ★`ClaudeStreamJson` = None★: `encode` 가 붙이는 종단 `\n` 이 그 프로토콜의 제출이다. 여기에 CR 을
    ///   더하면 라인 경계 뒤에 잉여 바이트가 붙어 페이로드가 오염된다.
    /// ★키 입력 경로는 이 값을 보지 않는다★: 소비자는 "완성된 메시지 하나 = 턴 하나" 인
    ///   `AgentSession::submit_input_observed` 뿐이다. 사람이 Enter 를 직접 치는 터미널 스트리밍 입력
    ///   (`write_input`)에 제출을 끼워 넣으면 키 한 번마다 턴이 제출된다.
    // ADR-0004
    pub fn submit_sequence(&self) -> Option<&'static [u8]> {
        match self {
            InputEncoder::Raw => Some(b"\r"),
            InputEncoder::ClaudeStreamJson => None,
        }
    }
}

/// 본문 write 와 제출 write([`InputEncoder::submit_sequence`]) **사이에 두는 대기**.
///
/// ★왜 필요한가(실측 2026-08-17 — 지우지 마라)★: write 를 둘로 나누는 것만으로는 제출되지 않는다. 간격이
///   없으면 두 write 가 PTY 에서 한 덩이로 묶여 claude TUI 가 **한 번의 read** 로 받고, CR 을 붙여넣은
///   텍스트의 일부로 취급해 입력창에 그대로 담아 둔다. 경계를 만드는 건 write 횟수가 아니라 **수신자의
///   read 경계**이고, 그걸 벌리는 수단이 이 대기다.
/// ★"지연 0 으로도 제출된다" 는 옛 관측은 측정 오류였다(되살리지 마라)★: 그 측정은 제어 평면 명령을 **두
///   번 따로** 보낸 것이라 웹소켓 왕복·태스크 전환으로 이미 수 밀리초가 벌어져 있었는데 0ms 로 읽혔다.
///   같은 함수에서 연달아 쓰는 배달 경로에는 그 간격이 없어 실제로 제출되지 않았다(살아 있는 에이전트로
///   재확인).
/// ★값의 출처 = 임의 상수가 아니다(ADR-0038)★: 같은 문제를 푸는 tmux 기반 멀티에이전트 오케스트레이터들이
///   claude Code 를 상대로 쓰는 정착값이 0.5초이고, 사유도 같다("텍스트와 Enter 를 합치거나 너무 붙여
///   보내면 입력 버퍼와 경쟁이 난다"). 근거 없이 줄이거나 늘리지 말 것 — 줄이면 이 결함이 그대로 재발한다.
pub const SUBMIT_PACING: std::time::Duration = std::time::Duration::from_millis(500);

pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    backend_for(c).input_encoder(c)
}

// ── 출력 정제(ADR-0044/0004/0045) — 입력 인코더의 대칭 짝 ──────────────────────────

/// pump→core 앞에 꽂히는 출력 정제 decoder. None = 바이트 직통(터미널·평문 불변).
///
/// ★대칭★: `input_encoder`(입력 방향)의 출력 방향 짝이다. 판정도 decoder 실물도
/// [`AgentBackend::output_decoder`] 가 소유하고 이 함수는 dispatch 뿐이다 — session 은 encoder 태그만,
/// transport 는 `dyn OutputDecoder` 만 안다(ADR-0004). 새 backend 는 자기 폴더에서 그 메서드를 구현하면
/// 되고 이 함수는 손대지 않는다(교체성).
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    backend_for(c).output_decoder(c)
}

// ── ADR-0079: resume 시 `.jsonl` transcript → 과거 이벤트 seed (backend dispatch) ──────

/// ADR-0079: resume 스폰 시 이 명령의 과거 대화를 복원한 `OutputEvent` 목록. 빈 Vec = seed 안 함
/// (기존 fresh 버퍼 동작 불변).
///
/// ★backend 지식 격리(ADR-0004)★: transcript 의 파일 배치·포맷·seed 가부 판정은 전부
///   [`AgentBackend::resume_transcript_events`] 구현체 몫이고 이 함수는 dispatch 뿐이다. manager 도 이
///   dispatch 만 부르고 경로 규칙을 모른다. `output_decoder`(라이브 정제)의 resume 방향 짝.
pub fn resume_transcript_events(
    c: &AgentCommand,
    cwd: &std::path::Path,
    session_id: Uuid,
) -> Vec<crate::types::OutputEvent> {
    backend_for(c).resume_transcript_events(c, cwd, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AgentOutputFormat;

    // ── ADR-0099 트립와이어: 새 AgentCommand variant 배선 시 capability 의식적 선언 강제 ──────
    //
    // ★ 와일드카드를 절대 추가하지 말 것 ★ — 새 AgentCommand variant가 생기면 이 match가
    // 컴파일 에러를 내서 아래 체크리스트를 강제로 방문하게 하는 장치다(목록 rot 방지 —
    // 갱신이 컴파일로 강제되는 목록은 rot하지 않는다).
    //
    // 새 variant 배선 시 체크리스트:
    //   ① 새 백엔드의 두 capability는 stub 복붙 금지 — CLI spike로 실측한 값으로 채울 것.
    //   ② supports_control_channel=false로 연결하면 메시징 없는 단독 에이전트가 된다(의도인지 확인).
    //   ③ 비-MCP(accepts_mcp_config=false)면 provision이 CLI판 프라이밍·[Cli] grant를 자동 선택한다
    //      — `roundtrip-smoke --cli-only`로 실측.
    //   ④ MCP-capable이면 기본 roundtrip으로 실측.
    //   참조: ADR-0099.
    fn expected_channel_matrix(c: &AgentCommand) -> (bool, bool) {
        // (supports_control_channel, accepts_mcp_config) — CLI spike 실측값
        match c {
            AgentCommand::Claude { .. } => (true, true),
            AgentCommand::Shell { .. } => (false, false),
            // codex 는 MCP 를 쓸 수 있지만 그 주입이 전역 TOML 오버라이드라 **이 mcp-config 파일**을
            //   먹일 수는 없다(실측) — 두 칸의 뜻 차이는 `backend/codex/` 가 적는다.
            AgentCommand::Codex { .. } => (false, false),
        }
    }

    #[test]
    fn backend_channel_matrix_is_consciously_declared() {
        let variants: Vec<AgentCommand> = vec![
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];

        for c in &variants {
            let (expected_control, expected_mcp) = expected_channel_matrix(c);
            let actual_control = supports_control_channel(c);
            let actual_mcp = accepts_mcp_config(c);
            assert_eq!(
                actual_control,
                expected_control,
                "variant {:?}: supports_control_channel 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
            assert_eq!(
                actual_mcp,
                expected_mcp,
                "variant {:?}: accepts_mcp_config 불일치 — backend mod.rs 상단 expected_channel_matrix 체크리스트를 따라 capability를 의식적으로 선언할 것(ADR-0099)",
                c
            );
        }
    }

    // ── ADR-0113/0004: 턴 신호 분류자 dispatch(매핑은 백엔드 소유, 어휘는 공용) ──────────────

    #[test]
    fn turn_classifier_dispatch_maps_claude_events_and_silences_shell() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let delta = OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let done = OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        };
        let claude_classify = turn_classifier(&json);
        assert_eq!(claude_classify(&delta), Some(TurnSignal::Progress));
        assert_eq!(claude_classify(&done), Some(TurnSignal::Ended));
        // ★`Structured` 해석이 백엔드별인 이유의 회귀★: claude 는 입력 시점 유저 에코를 여기 싣기에
        //   진행으로 세지만, 매핑을 선언하지 않은 backend 는 같은 이벤트에 침묵해야 한다 — 안 그러면
        //   턴과 무관한 구조화 메타 라인이 종료 신호 없는 영구 "턴 중" 을 만든다.
        let meta = OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        };
        assert_eq!(claude_classify(&meta), Some(TurnSignal::Progress));
        let shell_classify = turn_classifier(&shell);
        for ev in [&delta, &done, &meta] {
            assert_eq!(
                shell_classify(ev),
                None,
                "매핑 미선언 backend 는 어떤 이벤트에도 신호를 내지 않는다(기본 = 침묵)"
            );
        }
    }

    // ── ADR-0172/0004: 이어받기 실패 분류 dispatch(문구 지식은 백엔드 소유) ──────────────────

    #[test]
    fn resume_failure_dispatch_reads_claudes_marker_and_stays_silent_elsewhere() {
        let claude = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let tail = "\u{1b}[?25hNo conversation found with session ID: 8b1c…\r\n";
        assert_eq!(
            resume_failure_kind(&claude, tail),
            Some(AgentFailureKind::NoConversationToResume)
        );
        assert_eq!(
            resume_failure_kind(&claude, "NO CONVERSATION FOUND with session ID: x"),
            Some(AgentFailureKind::NoConversationToResume),
            "대소문자는 외부 프로그램이 정하므로 우리 판정이 거기 매달리면 안 된다"
        );
        assert_eq!(
            resume_failure_kind(&claude, ""),
            None,
            "꼬리가 비면 단정하지 않는다 — 구조화 세션은 진단을 stderr 로 흘려 여기 안 온다"
        );
        assert_eq!(
            resume_failure_kind(&claude, "Error: EPERM"),
            None,
            "모르는 문구는 단정하지 않는다(호출자가 맥락 기본값으로 떨어뜨린다)"
        );
        assert_eq!(
            resume_failure_kind(&shell, tail),
            None,
            "선언하지 않은 backend 는 남의 문구를 읽지 않는다"
        );
    }

    #[test]
    fn input_encoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert_eq!(input_encoder(&term), InputEncoder::Raw);
        assert_eq!(input_encoder(&shell), InputEncoder::Raw);
        assert_eq!(input_encoder(&json), InputEncoder::ClaudeStreamJson);
    }

    #[test]
    fn raw_encoder_is_byte_identical() {
        let input = b"echo hi\r\n\x1b[A\x03";
        assert_eq!(
            InputEncoder::Raw.encode(input, Uuid::new_v4()),
            input.to_vec()
        );
    }

    // ── 우편 자격 트립와이어: 새 variant 는 분류를 **의식적으로** 선언해야 한다 ──────────────────
    //
    // ★와일드카드를 추가하지 말 것★ — 기본값이 true(fail-open)라 아무 선언 없이도 수신자가 되는데,
    // 이 축이 막는 건 **LLM 자유 텍스트가 명령으로 실행되는 것**이라 조용한 상속은 안전 문제가 된다.
    //
    // ★목록 누락도 잡힌다(그래서 슬롯을 쓴다)★: 손으로 채우는 Vec 하나였을 땐 새 variant 를 거기 넣는 걸
    // 잊으면 `reads_messages` 가 한 번도 안 불려 테스트가 초록인 채 fail-open 이 통과했다. 새 variant 를
    // 추가하면 ① `variant_slot` 의 match 가 컴파일 에러 → ② 슬롯 번호를 늘리면 `BACKEND_VARIANTS` 와
    // 샘플 배열 길이가 안 맞아 다시 컴파일 에러 → ③ 그래도 샘플을 안 채우면 아래 "슬롯 전부 채움" 단언이
    // 깨진다. 세 관문 중 하나는 반드시 걸린다.
    //
    // 분류 기준: 입력을 **읽고 해석하는** 에이전트(CLI 코딩 에이전트 등)면 true, 입력을 **실행**하는
    //   채널(셸·REPL 류)이면 false. 판단이 서지 않으면 false 로 두고 사용자에게 올린다.
    const BACKEND_VARIANTS: usize = 3;

    fn variant_slot(c: &AgentCommand) -> usize {
        match c {
            AgentCommand::Claude { .. } => 0,
            AgentCommand::Shell { .. } => 1,
            AgentCommand::Codex { .. } => 2,
        }
    }

    fn expected_reads_messages(c: &AgentCommand) -> bool {
        match c {
            AgentCommand::Claude { .. } => true,
            AgentCommand::Shell { .. } => false,
            // ★shell 과 같은 값이지만 사유가 다르다★ — 여기 false 는 「입력이 실행된다」가 아니라
            //   「턴을 관측할 수 없어 바쁜 때를 못 가린다」다(정본 = `backend/codex/`). 사유가 갈리므로
            //   나중에 여는 조건도 갈린다.
            AgentCommand::Codex { .. } => false,
        }
    }

    /// variant 당 최소 1개. claude 는 모드가 둘이라 둘 다 싣는다(같은 슬롯 — 중복은 허용).
    fn mail_eligibility_samples() -> Vec<AgentCommand> {
        let per_variant: [AgentCommand; BACKEND_VARIANTS] = [
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];
        let mut all = per_variant.to_vec();
        all.push(AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        all
    }

    #[test]
    fn mail_eligibility_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            assert_eq!(
                reads_messages(c),
                expected_reads_messages(c),
                "variant {c:?}: 우편 자격 불일치 — 위 expected_reads_messages 의 분류 기준을 따라 의식적으로 선언할 것"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "샘플이 안 닿은 variant 가 있다(그 variant 는 reads_messages 가 한 번도 안 불려 fail-open 이 그대로 통과한다): {covered:?}"
        );
    }

    // ── 코덱 축 트립와이어: 새 variant·새 출력 모드는 입출력 코덱을 의식적으로 선언해야 한다 ──────────
    //
    // ★와일드카드를 추가하지 말 것★ — 이 축의 기본값(`Raw` · decoder 없음)은 둘 다 fail-open 이라
    // 아무 선언 없이도 컴파일되고 조용히 초록이 된다. 그런데 구조화 stdin 을 요구하는 프로그램에 `Raw`
    // 가 물리면 그 에이전트는 **입력을 통째로 못 읽고**, 런타임엔 아무 신호도 안 난다. `AgentCommand`
    // variant 든 `AgentOutputFormat` 값이든 늘어나면 아래 match 가 컴파일 에러를 낸다.
    //
    // 채우는 법 — CLI spike 실측값으로:
    //   ① input_encoder — 그 프로그램이 stdin 을 무엇으로 읽나(감쌀 게 없으면 `Raw`)
    //   ② output_decoder 유무 — 출력이 구조화라 정제가 필요한가
    //   ③ transport_shape — 그 출력을 받으려면 터미널이어야 하나 파이프여야 하나
    //
    // ★③ 은 ①②와 붙어 다니지만 같은 축이 아니다★: 통로 모양은 잘못 골라도 컴파일되고 **런타임에도
    // 그럴싸하게 뜬다** — TUI 를 파이프로 받으면 화면이 깨지고, 줄단위 JSON 을 PTY 로 받으면 이스케이프가
    // 섞인다. 기본값(`Pty`)이 fail-open 이라 여기 열이 없으면 새 backend 가 조용히 그것을 물려받는다.
    //
    // ★이 표가 덮지 않는 것 = resume transcript seed★: `resume_transcript_events` 도 같은 fail-open
    //   기본값(빈 Vec)을 갖지만, 판정하려면 실제 transcript 파일이 있어야 해서 "seed 안 함" 과 "파일이
    //   없어서 빈 Vec" 이 여기서 구별되지 않는다. 그쪽 회귀망은 `backend/claude/` 의 transcript 단위
    //   테스트가 진다 — 여기 세 번째 열을 만들면 아무 것도 재지 않는 열이 하나 생길 뿐이다.
    fn expected_codec_axis(c: &AgentCommand) -> (InputEncoder, bool, TransportShape) {
        // (input_encoder, output_decoder 유무, transport_shape)
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (InputEncoder::Raw, false, TransportShape::Pty),
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (
                InputEncoder::ClaudeStreamJson,
                true,
                TransportShape::StdioNdjson,
            ),
            AgentCommand::Shell { .. } => (InputEncoder::Raw, false, TransportShape::Pty),
            // ★codex 두 모드가 한 행에 묶인 것은 의도다★ — 오늘은 어느 모드로 띄워도 대화형 TUI 가 PTY
            //   위에서 돈다. JSON 모드가 쓸 통로가 아직 없기 때문이다. 번역기도 없다: 터미널 바이트가
            //   그대로 xterm 으로 간다. 그 통로가 붙으면 이 행을 모드별로 갈라야 하는데, ★그때 갈라지도록
            //   강제하는 테스트도 게이트도 없다★ — 통로를 들이는 쪽이 이 행을 직접 손봐야 한다.
            AgentCommand::Codex { .. } => (InputEncoder::Raw, false, TransportShape::Pty),
        }
    }

    #[test]
    fn codec_axis_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            let (expected_encoder, expects_decoder, expected_shape) = expected_codec_axis(c);
            assert_eq!(
                input_encoder(c),
                expected_encoder,
                "variant {c:?}: input_encoder 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
            assert_eq!(
                output_decoder(c).is_some(),
                expects_decoder,
                "variant {c:?}: output_decoder 유무 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
            assert_eq!(
                transport_shape(c),
                expected_shape,
                "variant {c:?}: transport_shape 불일치 — 위 expected_codec_axis 를 따라 의식적으로 선언할 것"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "샘플이 안 닿은 variant 가 있다(그 variant 는 코덱 축이 한 번도 안 불려 fail-open 이 그대로 통과한다): {covered:?}"
        );
    }

    // ── 선언한 통로 모양과 실제로 넘어오는 통로 ────────────────────────────────────────────
    //
    // ★왜 위 코덱 축 표와 따로 재나(ADR-0191 이후)★: 조립점이 `transport_shape` 를 match 해 생성자를
    //   고르던 시절엔 그 표의 셋째 열이 **실제로 뜨는 통로**까지 전이적으로 단언했다. 통로 생성이
    //   backend 의 `open_spawn` 으로 들어가면서 그 신고값을 읽는 생산 코드가 0 이 됐고, 그때부터
    //   파이프를 선언해 놓고 기본 `open_spawn`(=PTY)을 타도 그 열은 초록이다. 런타임 증상은 깨진
    //   화면뿐이라 지금 그 어긋남을 잡는 것은 이 테스트뿐이다.
    // ★`structured` 로 가르지 않는 이유★: 그 칸은 통로가 아니라 backend 가 주입하는 값이라
    //   (ADR-0030/0044) 평문 파이프도 false 를 신고한다 — 두 구현체를 실제로 가르는 것은 양쪽이
    //   하드코딩하는 `terminal_bytes` 와 `resize` 다.
    // ★슬롯 커버리지 단언을 두지 않았다★: 같은 샘플 목록을 도는 위 두 트립와이어가 이미 잰다.
    // ★spec 은 그 백엔드의 실 CLI 가 아니다★: `open_spawn` 은 무엇을 띄울지를 spec 에서, 어떤 통로로
    //   띄울지를 command 에서 따로 받으므로, 즉시 끝나는 프로브를 띄워 통로 선택만 본다(ADR-0012).
    // ADR-0191
    #[cfg(windows)]
    #[test]
    fn declared_transport_shape_matches_the_transport_handed_over() {
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo shape-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        for c in &mail_eligibility_samples() {
            let shape = transport_shape(c);
            let expected = match shape {
                TransportShape::Pty => (true, true),
                TransportShape::StdioNdjson => (false, false),
            };
            let parts = open_spawn(c, &probe, 80, 24).expect("open_spawn");
            let caps = parts.transport.capabilities();
            let actual = (caps.output.terminal_bytes, caps.control.resize);
            parts.transport.shutdown();
            assert_eq!(
                actual, expected,
                "variant {c:?}: {shape:?} 를 신고했는데 open_spawn 이 넘긴 통로의 (terminal_bytes, resize) 가 다르다 — 신고와 통로 생성 중 한쪽만 고친 것이다"
            );
        }
    }

    // ── 제출 바이트(submit_sequence) — 백엔드별 "본문 write 만으론 턴이 안 시작되나" ──────────────
    #[test]
    fn submit_sequence_is_cr_for_raw_and_absent_for_stream_json() {
        assert_eq!(
            InputEncoder::Raw.submit_sequence(),
            Some(b"\r".as_slice()),
            "터미널·shell 은 PTY 키보드 흉내 — Enter = CR"
        );
        assert_eq!(
            InputEncoder::ClaudeStreamJson.submit_sequence(),
            None,
            "encode 의 종단 \\n 이 이미 제출 — CR 을 더하면 라인 뒤에 잉여 바이트가 붙는다"
        );
    }

    #[test]
    fn submit_sequence_is_not_baked_into_encode() {
        // ★합치면 claude TUI 가 제출하지 않는다(실측)★ — encode 산출물에 제출 바이트가 섞여 들어가면
        //   session 이 두 write 로 나눠도 첫 write 가 이미 오염된 상태다.
        let out = InputEncoder::Raw.encode(b"<message from=\"bob\">hi</message>", Uuid::new_v4());
        assert!(
            !out.contains(&b'\r'),
            "Raw encode 는 제출 바이트를 붙이지 않는다(제출은 write 경계 — session 소관): {out:?}"
        );
    }

    // ── ADR-0044/0045: 입력-시점 유저 에코 이벤트 dispatch(input_echo_event) — uuid dedup ──────
    #[test]
    fn input_echo_event_json_mode_emits_structured_user_with_uuid() {
        use crate::types::OutputEvent;
        let id = Uuid::new_v4();
        let ev = InputEncoder::ClaudeStreamJson
            .input_echo_event(b"hi there", id)
            .expect("json 모드 → 합성 유저 에코 이벤트");
        match ev {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "hi there");
                assert_eq!(
                    v["uuid"],
                    id.to_string(),
                    "합성 에코에 msg_uuid 부착(dedup 키)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn input_echo_event_raw_is_none() {
        assert!(
            InputEncoder::Raw
                .input_echo_event(b"echo hi\r\n", Uuid::new_v4())
                .is_none(),
            "Raw 는 합성 유저 에코를 만들지 않아야 함(PTY 에코 중복 방지)"
        );
    }

    #[test]
    fn claude_stream_json_encoder_wraps_and_terminates() {
        let id = Uuid::new_v4();
        let out = InputEncoder::ClaudeStreamJson.encode(b"hi", id);
        assert_eq!(*out.last().unwrap(), b'\n');
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"text\":\"hi\""));
        assert!(
            s.contains(&id.to_string()),
            "stdin user 라인에 msg_uuid 포함"
        );
    }

    // ── S15 B3: output_decoder dispatch(입력 encoder 의 대칭) ──────────────────────
    #[test]
    fn output_decoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert!(
            output_decoder(&term).is_none(),
            "터미널 모드 → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&shell).is_none(),
            "shell → decoder 없음(직통)"
        );
        assert!(
            output_decoder(&json).is_some(),
            "json 모드 → ClaudeStreamDecoder 주입"
        );
    }

    #[test]
    fn output_decoder_produces_structured_events_through_trait_object() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let mut dec = output_decoder(&json).expect("json → decoder");
        let mut ev = dec.decode(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        ev.extend(dec.flush());
        assert!(
            ev.iter()
                .any(|e| matches!(e, OutputEvent::MessageDone { .. })),
            "trait object decode 가 result 라인을 MessageDone 으로 정제해야 함: {ev:?}"
        );
    }
}
