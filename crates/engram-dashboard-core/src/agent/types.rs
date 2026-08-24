pub type AgentId = uuid::Uuid;

pub type SinkId = uuid::Uuid;

/// internally-tagged 직렬화 — 프론트가 discriminated union 으로 받는다.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

impl AgentStatus {
    /// ★"세션 맵에 있음" 과 다르다(load-bearing)★: 세션은 reaper 가 수거할 때까지 맵에 남으므로
    ///   단순 존재로 판정하면 시체가 섞인다. 명부(`AgentManager::roster`)와 데몬 어댑터
    ///   (`messaging_host::is_live` — 이 술어를 호출만 한다)가 **같은 조건**을 봐야 발송 측과
    ///   flush 측이 다른 세계를 보지 않는다. 이 술어가 정본 — 복제본을 만들지 말 것.
    // ADR-0116
    pub fn is_live(&self) -> bool {
        matches!(self, AgentStatus::Running | AgentStatus::Exiting)
    }
}

/// pump→core 내부 출력 이벤트. 확장 가능 enum. core는 variant-agnostic(_ => ignore).
///
/// ★ADR-0045 (출력 정제를 백엔드로)★: 콘솔은 `TerminalBytes`(VT 바이트 스트림) 그대로,
/// 구조화 백엔드(claude stream-json 등)는 backend decoder가 파싱해 아래 구조화 variant로 emit한다.
/// 이 타입은 **core 도메인 타입**이지 protocol wire 타입이 아니다 — core↔wire 변환은 daemon
/// adapter가 한다(ADR-0003 격리: core는 wire를 모른다). core에 tauri import 금지(serde는 허용).
///
/// `turn_id`/`message_id`는 대화 추적용 optional 필드다 — claude는 안 채워도 되고, codex/gemini의
/// turn·message 모델 누수를 흡수하려 열어 둔다(교체성). backend가 못 채우면 None.
#[derive(Debug, Clone)]
pub enum OutputEvent {
    /// 콘솔 raw 바이트(VT 스트림). PtyTransport·터미널 모드의 유일 payload.
    TerminalBytes(Vec<u8>),
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// `args_json` = backend 스키마 그대로의 직렬화 인자(core 무정제).
    ToolCall {
        name: String,
        args_json: String,
        /// 호출 식별자(권한 UX·결과 매칭용). claude tool_use id 등.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// 한 메시지(turn 응답) 종료 신호.
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// backend가 보고한 오류(스트림 내부 오류 등 — TerminalReason과 별개, 종료 아님).
    Error(String),
    /// 위 정형 variant로 안 잡히는 backend별 구조화 이벤트의 탈출구(forward-compat).
    /// kind=이벤트 종류 태그, json=원본 직렬화 payload. core는 내용을 해석하지 않는다.
    Structured { kind: String, json: String },
}

/// session→transport 입력 이벤트. 확장 가능 enum.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Raw(Vec<u8>), // PTY 키 입력 바이트
}

/// transport가 산출하는 종료 사유(flat). core가 AgentStatus로 매핑(finalize 1회).
/// ※ raw lib error(reqwest/nix) 직접 노출 금지 — 도메인 문자열로.
#[derive(Debug, Clone)]
pub enum TerminalReason {
    Exited { code: Option<i32> },
    Killed,
    Interrupted,
    StreamClosed,
    Cancelled,
    Error(String),
}

/// 유저 의도 — kill 핸들러가 채운다(ADR-0019). PTY 관측 사실(TerminalReason)과 **분리**한다:
/// 종료를 관측해 의도를 추론하면 데몬 셧다운 Job-kill 이 유저 kill 로 오분류되므로, 의도는
/// "종료를 일으킨 행동 지점"(kill 커맨드 핸들러)에서 명시적으로 태깅한다.
/// `#[repr(u8)]` — `Arc<AtomicU8>` 로 세션별 보관·snapshot 한다. DaemonShutdown 은 전역
/// `shutting_down` 플래그로 분리(여기 두지 않음).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationIntent {
    None = 0,
    UserKill = 1,
}

impl TerminationIntent {
    /// 알 수 없는 값은 보수적으로 None(= 크래시 취급 경로).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TerminationIntent::UserKill,
            _ => TerminationIntent::None,
        }
    }
}

/// pump 가 finish 승자일 때 1회 발행하는 종료 이벤트(ADR-0019 reaper 가 단일 소비).
///
/// ★race 방지 핵심★: `intent_at_finish`/`shutting_down_at_finish` 는 **finish 그 순간** snapshot
/// 한 frozen 값이다. reaper 가 reap 시점에 live 로 읽으면 "크래시로 죽은 뒤 reaper 처리 전 유저가
/// kill→크래시를 유저kill 로 오분류→프로필 삭제(데이터 손실)" race 가 생긴다(consult GPT 적출).
#[derive(Debug, Clone)]
pub struct ReapMsg {
    pub id: AgentId,
    /// stale done 이 재spawn 된 새 세션을 오삭제 못 하게 reap 전 epoch 일치 검증(ADR-0007).
    pub epoch: u32,
    pub reason: TerminalReason,
    pub intent_at_finish: TerminationIntent,
    pub shutting_down_at_finish: bool,
}

/// 종료 분류 결과(ADR-0019 §decide). reap_one 이 lock 밖에서 ProfileRegistry 에 적용한다.
/// ★삭제 처분이 없는 건 의도다(ADR-0083)★ — reaper 는 어떤 종료에도 프로필을 자동 삭제하지 않는다.
/// 프로필 삭제는 명시적 사용자 명령(AgentCommand::DeleteProfile / Tauri delete_profile)이
/// ProfileRegistry::remove 를 직접 호출할 뿐, 이 enum 을 거치지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// 모든 런타임 종료(유저 kill·정상 exit·크래시·EOF·signal) → 프로필 유지 + auto_restore=false
    /// (시체 보존 — 재활성화 시 --resume 로 이어받음).
    KeepDisableAutoRestore,
    /// 데몬 셧다운 → 손 안 댐(auto_restore=true 잔류 → 부팅 복원).
    KeepAsIs,
}

/// transport에 주입하는 중립 실행 명세. backend가 산출. PtyTransport는 claude/codex를 모름.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

// ── ADR-0086: 제어 채널 입구(MCP) — core seam ──────────────────────────────────────
//
// ★왜 core 에 추상 descriptor + seam 을 두는가★: 스폰되는 에이전트가 데몬의 제어 채널(MCP 입구)에
//   붙으려면 (a) 데몬이 (AgentId,epoch)별 토큰을 발급하고 (b) 그 토큰+엔드포인트를 backend 명령줄에
//   주입해야 한다. 그러나 **토큰 발급·MCP 서버·mcp-config 파일**은 전부 데몬 관심사(rmcp/axum/HTTP)라
//   core 에 들어오면 tauri-import-0 격리와 같은 정신(전송·인프라 무의존)이 깨진다. 그래서 OutputSink/
//   StatusSink 와 **동일한 idiom(ADR-0003)** 으로, core 는 순수 trait(`ControlChannel`) + 추상
//   descriptor(`ControlEndpoint`)만 알고 실제 구현은 데몬(`DaemonControlChannel`)이 준다.

/// 스폰 에이전트가 셸에서 **실제로 치는 bare 실행파일 이름**(경로·확장자 없음). 배포되는 실행파일의
/// stem, `ToolGrant::Cli` 의 exe, backend 가 PATH 에 붙여 해석시키는 이름, 프라이밍이 가르치는 명령 —
/// 넷이 **글자 그대로 같아야** 한다(ADR-0094 정렬 불변식).
///
/// ★어디서도 다시 타이핑하지 말 것★: 이름이 한 자리라도 갈라지면 에이전트는 PATH 에 없거나 grant 에
///   안 걸린 명령을 부르고, 우편은 에러 없이 **조용히 멈춘다**(ADR-0099 실측: 7건 중 6건 미발신).
/// ★지금 CI 가 잡는 것 / 못 잡는 것★: grant 문자열·PATH 해석 이름은 이 상수에서 파생돼 따로 어긋날 수
///   없고, **배송 파일명 ↔ 상수**(daemon `tests/engram_cli.rs`)와 **프라이밍 ↔ 상수**(daemon
///   `control/priming.rs` pin)는 claude 없이 도는 테스트라 CI 가 본다. CI 밖에 남는 축은 **에이전트가 실제로
///   그 이름을 해석해 실행하는지**다 — 실 claude 스폰 테스트가 러너에 claude 가 없어 제외되기 때문이고,
///   그건 로컬 실측으로만 확인된다.
// ADR-0094
pub const CLI_EXE_NAME: &str = "engram";

/// CLI 우편 계열 이름 — `engram mail <동사>` 의 가운데 토큰(ADR-0132 그룹 구조).
///
/// ★왜 core 가 이걸 아는가★: 우편 채널의 **교육↔배선 등호**(ADR-0128/0099)를 지키는 판정자들이 프라이밍
///   **본문에서 우편 CLI 교육을 찾아내야** 하는데, 그 판정 토큰이 CLI 가 실제로 받는 표기와 갈리면 판정이
///   조용히 뒤집힌다. 그래서 CLI 의 디스패치와 그 판정자들이 같은 값을 본다.
/// ★bare 실행파일 이름만으로는 그 판정을 할 수 없다★: MCP 프라이밍이 같은 단어를 MCP **서버 이름**으로
///   정당하게 쓴다(`on the engram server`) — 그래서 판정은 `CLI_EXE_NAME` + 이 토큰의 **인접**으로 한다.
// ADR-0132
pub const CLI_GROUP_MAIL: &str = "mail";

/// CLI 가 제어 라우트에서 견디는 **침묵의 한도**(초) — 데몬 마감의 상한이다.
///
/// ★「총 예산」이 아니다★: 소비자는 이것을 `set_read_timeout`(+`set_write_timeout`)에 넣으므로, 발동
/// 조건은 「요청을 보낸 뒤 **한 번의 read 가** 이 시간 안에 바이트를 하나도 못 받는 것」이다 — 왕복 전체의
/// 소요가 아니라 **연속 무응답 구간**을 잰다. 오늘 제어 라우트가 답 전에 아무 바이트도 안 흘리므로 두
/// 값이 사실상 같지만, 그 우연에 기대는 문장을 쓰지 말 것(중간 바이트가 생기면 총 소요는 이 값을 넘어도
/// 클라이언트는 안 끊는다).
/// ★두 쪽이 같은 값을 봐야 하는 이유★: 데몬이 마감을 넘긴 왕복에 `TIMEOUT` 을 **실어 답하는데**, 그 답이
/// 나가기 전에 CLI 가 소켓을 끊으면 그 답은 아무도 못 본다 — 사용자에게는 「데몬에 닿지 못했다」로 보이고
/// (실제로는 닿았고 명령이 적용됐을 수도 있다) 데몬 쪽에는 답장을 낼 곳이 사라진다. 실제로 그렇게
/// 뒤집혀 있었다: 양쪽이 각자 10초를 들고 있는데 CLI 의 시계가 **먼저** 시작하고(연결·파싱·풀 스케줄링이
/// 데몬 마감 앞에 있다) 데몬의 답은 수거 주기(1초) 뒤에야 나가, 클라이언트가 결정적으로 먼저 끊었다.
/// ★그래서 관계는 부등식이다★: `데몬 마감 + 수거 주기 + 여유 ≤ 이 값`. 그 부등식을 무는 자리는 데몬
/// 쪽 하나다(daemon `command_delivery` 의 `fits_caller_silence_window` — 기본값은 `const` 단언으로,
/// 주입값은 생성자에서 판정한다). 이 값을 줄이면 그 단언이 깨져 빌드가 멈춘다. **여기서 그 셈을 다시
/// 적지 말 것** — 두 사본이 갈리는 날 어느 쪽도 못 믿는다.
/// ★초 단위 정수인 이유★: `Duration` 은 core 의 이 목록이 드는 다른 CLI 어휘와 결이 다르고, 두 소비자가
/// 각자 자기 타입으로 감싸는 편이 이 상수를 순수한 **숫자**로 남긴다.
pub const CLI_CONTROL_READ_TIMEOUT_SECS: u64 = 10;

/// CLI 실행파일의 **절대경로**를 스폰 env 로 실어 보낼 때 쓰는 변수 이름.
///
/// backend 가 이 이름으로 값을 넣고(claude), 프라이밍 판정자들은 본문에서 이 이름을 찾아 "CLI 표면이
/// 적혀 있는가" 를 본다 — 두 자리가 같은 값을 봐야 판정이 실제 배선과 어긋나지 않는다.
/// ★이름만 등장하는 것은 교육이 아니다★: 발신 교육으로 세는 것은 **호출 형태**(이 변수 뒤에 계열 토큰이
///   붙는 형태)뿐이다. 판정 규칙의 정본은 daemon `control/priming.rs`.
// ADR-0132
pub const CLI_EXE_ENV: &str = "ENGRAM_CLI_EXE";

/// 스폰 시 실어 보내는 **CLI 우편 계열 가부 표식**의 변수 이름과 두 값(ADR-0133 결정 2).
///
/// ★`off` 는 "우편 금지" 가 아니라 "이 입구가 네 채널이 아니다" 다★: 그 스폰은 MCP 툴로 우편을 쓴다
///   (ADR-0128 결정 1 — 채널은 백엔드 capability 로만 갈린다).
/// ★이것은 교육 수단이지 강제가 아니다★: CLI 는 이 값을 읽어 **사용법 목록에서 우편 계열을 감출 뿐**이고,
///   실제 발송 허용 여부는 데몬이 자격증명으로 판정한다. 표식은 에이전트 자신의 프로세스에 붙어 조작
///   가능하며, 조작을 수용한다 — 표식을 떼면 목록에 우편이 보이지만 발송은 여전히 거절된다.
///   **표식 필터에 강제를 의존하는 구현은 위반이다**: 표식을 떼는 순간 우편이 열린다.
/// ★부재·모르는 값 = 전부 보인다(fail-open, 의도)★: 사람이 스폰 밖 셸에서 `engram help` 를 열었을 때
///   사용법이 반쪽으로 나오면 안 된다. 숨김은 교육이지 권한이 아니므로 모르는 값에 fail-closed 할 이유가
///   없다.
// ADR-0133
pub const MAIL_MARKER_ENV: &str = "ENGRAM_MAIL";
pub const MAIL_MARKER_ON: &str = "on";
pub const MAIL_MARKER_OFF: &str = "off";

/// 우편 계열의 동사 전량 — `engram mail <동사>`.
///
/// 세 소비자가 같은 값을 봐야 한다: CLI 파서(무엇을 받나) · 프라이밍 판정자(본문이 **실행 가능한** 호출을
/// 가르쳤나) · 사용자 안내 문구. 판정자가 계열 토큰까지만 보면 `engram mail` 처럼 **동사 없는 조각**도
/// 교육으로 세는데, 그건 실행되지 않는 명령이라 "가르쳤다" 가 거짓이 된다.
// ADR-0132
pub const CLI_MAIL_VERBS: [&str; 3] = ["send", "status", "pending"];

/// 우편 CLI 전용 플래그 표기(kebab) 전량.
///
/// 파서가 인식하는 집합이자, 프라이밍 판정자가 "CLI 표면이 적혀 있나" 를 보는 어휘다. MCP 입구는 같은
/// 개념을 snake_case JSON 필드(`reply_to`)로 받으므로 **표기 축이 두 입구를 가른다**.
/// ★파서의 match arm 과 이 목록이 갈리면★ 값 자리 방어(플래그를 값으로 삼키는 사고)와 프라이밍 판정이
///   새 플래그를 못 본다 — daemon `bin/engram.rs` 의 드리프트 테스트가 그 어긋남을 잡는다.
// ADR-0132
pub const CLI_MAIL_FLAGS: [&str; 6] = [
    "--to",
    "--body",
    "--body-stdin",
    "--request",
    "--reply-by",
    "--reply-to",
];

/// CLI 제어 계열 이름 — `engram agent <동사>` 의 가운데 토큰.
///
/// ★우편 계열(`CLI_GROUP_MAIL`)과 달리 프라이밍 판정에 쓰이지 않는다★: 우편은 "가르친 채널 = 깐 배선"
///   등호(ADR-0128)를 판정자가 지켜야 해서 core 가 그 토큰을 알아야 했지만, 제어는 그런 등호가 없다
///   (ADR-0132 결정 5 — 권한은 전원 개방). 여기 있는 이유는 **CLI 파서·help·드리프트 테스트가 한 문자열을
///   보게** 하는 것뿐이다.
// ADR-0132
pub const CLI_GROUP_AGENT: &str = "agent";

/// 제어 계열의 동사 전량 — `engram agent <동사>`.
///
/// ★`kill`·`rm` 이 없는 것은 미구현이 아니라 **보류된 결정**이다★: 트리에서 지우는 것이 에이전트의 생을
///   끝내는가(ADR-0122)가 아직 코드와 어긋나 있어(현 `delete_agent` 는 프로필만 지우고 프로세스를 남긴다)
///   그 둘을 여기 얹으면 새 입구로 그 불일치가 노출된다. 그 결정이 서기 전에는 이 목록에 넣지 말 것.
// ADR-0132
// ADR-0122
pub const CLI_AGENT_VERBS: [&str; 5] = ["list", "spawn", "new", "rename", "move"];

/// 제어 계열 전용 플래그 표기(kebab) 전량 — 파서가 인식하는 집합이자 값 자리 방어의 어휘다
/// (`CLI_MAIL_FLAGS` 와 같은 규율, 계열별로 따로 둔다 — 한 계열의 플래그가 다른 계열 값 자리를 가로채면
/// 안 되기 때문이다).
// ADR-0132
pub const CLI_AGENT_FLAGS: [&str; 3] = ["--cwd", "--name", "--parent"];

/// 제어 응답이 싣는 **에이전트 상태 축**(살아 있음·잠듦·없음)의 wire 표기.
///
/// ★왜 core 에 있나★: 이 문자열의 생산자(`agent::commands` 의 표)와 소비자(CLI 의 응답 판정기)가 **다른
///   crate** 라, 각자 리터럴을 적으면 한쪽만 바뀌어도 아무도 못 본다 — 증상은 정상 응답이 "읽을 수 없는
///   shape"(exit 2)로 튀는 거짓 경보다. 한 값을 양쪽이 본다.
/// ★"없음" 에 해당하는 값이 없는 것은 의도다★ — 부재는 상태값이 아니라 반려 코드(`NOT_FOUND`)로
///   표현된다. 메시지 결말 어휘(`delivered`/`pending`/`failed`)와 섞지 않는다(ADR-0116).
// ADR-0132
pub const AGENT_STATE_LIVE: &str = "live";
pub const AGENT_STATE_SLEEPING: &str = "sleeping";

/// 개명 응답의 `outcome` 어휘 — 위와 같은 이유로 core 가 소유한다. 실패 두 갈래(부재·이름 공간 소진)는
/// 반려 코드로 나가므로 여기 없다.
// ADR-0132
pub const RENAME_OUTCOME_RENAMED: &str = "renamed";
pub const RENAME_OUTCOME_UNCHANGED: &str = "unchanged";

/// ADR-0094: 스폰 에이전트에 **사전 승인**할 툴 1개의 추상 명세(backend-agnostic). 데몬 컨트롤 채널이
/// 자기 입구 정의 옆에서 채우고, backend 가 자기 프로그램 문법으로 번역한다(claude = `--allowedTools`).
///
/// ★왜 추상 enum 인가(단일 출처·격리)★: 발신 입구의 **정체**(어느 MCP 서버의 어느 툴, 어느 CLI exe)는
///   컨트롤 채널만 안다 — 툴 이름(`send_message`)·서버명(`engram`)·CLI 경로는 그쪽 정의가 정본이다.
///   core 는 그 정체를 데이터(server/tool/exe 문자열)로만 나르고 "권한"·"allowlist" 개념을 모른다.
///   backend/claude.rs 는 이 데이터를 claude 문법(`mcp__{server}__{tool}` / `Bash({exe}:*)` +
///   `PowerShell({exe}:*)`)으로만 번역한다 — 이름을 재타이핑하지 않는다(ADR-0004 격리 + ADR-0094 단일 출처 불변식).
/// ★최소권한(ADR-0094)★: 이 목록엔 발신 입구 툴만 담긴다 — 이 *목록*을 넓히려면 명시적 결정(ADR-0094 개정).
///   주의: 2026-07-22 사용자 결정으로 스폰 자체는 `--permission-mode bypassPermissions`(auto) 하에 돈다 —
///   이 grant 는 지금 런타임 게이트가 아니라 **미래 공용 제약 레이어용 정책 표면 + 문서화**로 남는 것이다
///   (backend/claude.rs 참조, step-log 백로그 "전 LLM 공용 제약 레이어").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGrant {
    /// MCP 서버의 툴 1개. backend 가 `mcp__{server}__{tool}` 로 번역한다(claude).
    Mcp { server: String, tool: String },
    /// CLI 실행 파일 1개(그 exe 로 시작하는 명령을 허용 — bare 이름, backend 주입 PATH 로 해석).
    /// backend 가 `Bash({exe}:*)` **와** `PowerShell({exe}:*)` 두 패턴으로 번역한다(claude — 두 shell 도구 모양).
    Cli { exe: String },
}

/// 데몬이 발급하는 제어 채널 엔드포인트(추상 descriptor). backend 가 이걸 받아 자기 프로그램의
/// 방식으로 명령줄/env 에 주입한다(claude = `--mcp-config <path>` — 그 지식은 backend/claude.rs 단독,
/// ADR-0004). core/transport 는 url/token/path 문자열만 나르고 "MCP" 나 claude 플래그를 모른다.
#[derive(Debug, Clone)]
pub struct ControlEndpoint {
    /// 데몬 MCP Streamable HTTP 엔드포인트 URL(예: `http://127.0.0.1:<port>/mcp`).
    pub url: String,
    /// 이 (AgentId,epoch) 전용 bearer 토큰(HTTP Authorization 헤더에 실린다).
    /// ★보안★: 이 값은 로그에 찍지 않는다(mcp-config 파일에만 기록 — 파일은 revoke 시 삭제).
    pub token: String,
    /// 에이전트별 mcp-config 파일 경로(데몬이 만들고 revoke 시 지운다). backend/claude.rs 가 이 파일에
    /// url+token 을 써서 `--mcp-config` 로 주입한다.
    /// ★Option = 부재를 타입으로 인코딩(ADR-0099)★: MCP-capable 백엔드(claude)면 `Some(path)`(mcp-config
    ///   물리 존재), 비-MCP 백엔드(codex/gemini stub)면 `None` — mcp-config 를 **아예 쓰지 않는다**(MCP
    ///   입구 물리 삭제). backend 는 `Some` 일 때만 `--mcp-config` 를 주입한다(빈 경로 방어 코드 불필요 —
    ///   타입이 강제).
    /// ★이 Option 은 **MCP 입구의 유무**만 뜻한다(ADR-0133)★ — CLI 배선의 유무가 아니다. CLI 는 두 갈래
    ///   전원에게 깔리고, 우편 가부는 `mail_allowed` 표식(교육)과 데몬 거절(강제)이 가른다. 데몬이
    ///   MCP-capable 일 때만 config 를 쓰고 그 write 실패 시 provision 을 Err 로 끊으므로 반쪽 MCP endpoint
    ///   는 존재하지 않는다.
    pub config_path: Option<std::path::PathBuf>,
    /// ADR-0086 스텝 2(CLI 입구): 데몬이 위치를 찾아낸 `engram` CLI 바이너리 절대경로(있으면).
    /// 데몬 exe 의 형제라 배포 시 동거하나, 부분 빌드 등으로 없을 수 있다 → `None` 이면 backend 가
    /// 그 env(claude=`ENGRAM_CLI_EXE`)와 PATH 프리펜드를 주입하지 않는다.
    /// ★소비 조건 = control endpoint 가 있는 스폰 전부(ADR-0133 · ADR-0132 결정 5)★: 제어 동사는 전원에게
    /// 열리므로 두 갈래 모두 이 값을 쓴다. 형제 exe 탐색 지식은 데몬 소유(lib.rs).
    pub send_exe: Option<std::path::PathBuf>,
    /// 이 스폰이 **CLI 우편 계열(`engram mail …`)을 쓸 수 있는가**(ADR-0133 결정 2). backend 가 표식
    /// env(`MAIL_MARKER_ENV`)로 번역해 싣고, CLI 는 그 값을 읽어 사용법에서 우편 계열을 보이거나 감춘다.
    ///
    /// ★이름보다 좁다 — "우편을 쓸 수 있는가" 가 아니다★: `false` 인 스폰(= MCP 가능 백엔드)은 MCP 툴로
    ///   **우편을 정상적으로 쓴다**. 채널은 백엔드 capability 로만 갈리고 런타임 폴백이 없다(ADR-0128
    ///   결정 1) — 이 값은 그 설계에서 닫혀 있어야 할 쪽 입구(CLI/HTTP)만 가리킨다.
    ///
    /// ★core 는 정책을 파생하지 않는다★: 이 값은 데몬이 판정해 실어 주는 **사실**이고, backend 는 실린
    ///   대로 주입만 한다. `config_path` 유무 같은 다른 필드에서 이 값을 다시 유도하지 말 것 — 두 곳이
    ///   갈리면 교육과 강제가 어긋난다.
    /// ★교육 수단이지 강제가 아니다★: 강제는 데몬이 자격증명으로 하는 거절뿐이다(`MAIL_MARKER_ENV` 주석).
    // ADR-0133
    pub mail_allowed: bool,
    /// ADR-0092(수신 계약 프라이밍): 스폰 시 시스템 프롬프트에 주입할 **프라이밍 MD 파일의 절대경로**
    /// (있으면). 데몬의 `PrimingProvider` seam 이 해석해 실어 보낸다 — 파일 부재/미구성이면 `None`.
    /// backend/claude.rs 가 이 경로를 `--append-system-prompt-file <abs-path>` 로 주입한다(claude 가
    /// 파일을 **직접 읽음** — 데몬/core 는 내용을 안 읽는다). MCP 와 직교하는 broker-주입 데이터지만,
    /// 데몬이 이미 모든 claude 스폰에 대해 채우는 이 descriptor 를 재사용해 별도 threading 경로를 만들지
    /// 않는다.
    pub priming_file: Option<std::path::PathBuf>,
    /// 사전 승인할 툴 목록(ADR-0094 — 계약은 `ToolGrant`). 데몬 컨트롤 채널이 발신 입구(MCP
    /// `send_message` / `engram` CLI)를 채운다. 빈 Vec 이면 backend 가 아무 것도 주입하지 않는다
    /// (권한 플래그 없음 = 기존 게이트 유지).
    pub grants: Vec<ToolGrant>,
    /// S18 D(spec §6 allowedMcpServers 대책): 스폰 세션에만 얹을 **설정 조각 파일의 절대경로**(있으면).
    /// 데몬이 provision 때 `<data_dir>/mcp-config/<id>-<epoch>.settings.json` 에 쓰고 revoke 때 지운다.
    /// backend/claude.rs 가 `--settings <abs-path>` 로 번역한다(그 플래그 지식은 거기 단독 — ADR-0004).
    ///
    /// ★왜 필요한가(실측 2026-07-24)★: 유저 전역 설정의 `allowedMcpServers: []`(= 전면 차단)가 **스폰
    ///   에이전트에도 그대로 적용**돼 engram MCP 서버가 툴 목록에 뜨지 않았다. 이 조각이 그 세션에만
    ///   engram 서버를 허용한다 — **전역 설정 파일은 절대 건드리지 않는다**(허용 범위 = 엔그램이 스폰한
    ///   에이전트뿐). config_path 와 같은 수명(epoch 단위 생성·폐기)이라 같은 descriptor 에 태운다.
    pub settings_file: Option<std::path::PathBuf>,
}

/// 제어 채널 provision 실패 사유(ADR-0086 fail-closed). 파일 write·CSPRNG 실패 등 "제어 채널을 붙일
/// **의도가 있었으나 실패**"한 경우다 — spawn 이 이 Err 를 만나면 fail-closed 로 스폰을 중단한다(제어
/// 채널 없이 도는 에이전트를 만들지 않는다). ★absence 와 구분★: Ok(None)=제어 채널을 안 쓰는 정당한
/// 부재(Noop·shell), Err=쓰려다 실패(치명). core 는 문자열만 나른다(rmcp/io 타입 누수 방지, ADR-0003).
#[derive(Debug)]
pub struct ProvisionError(pub String);

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "control channel provision failed: {}", self.0)
    }
}

impl std::error::Error for ProvisionError {}

/// 제어 채널 provisioning seam(ADR-0086). AgentManager 기본값 = `NoopControlChannel`.
pub trait ControlChannel: Send + Sync + 'static {
    /// (AgentId,epoch)용 토큰을 발급하고 (MCP-capable 이면) mcp-config 파일을 만들어 엔드포인트를 돌려준다.
    /// spawn 경로에서 spec 조립 직전 호출. 반환 3-값(fail-closed 계약, ADR-0086):
    ///   - `Ok(Some(ep))` — 제어 채널 발급 성공(backend 가 주입).
    ///   - `Ok(None)`     — 제어 채널을 **안 쓰는 정당한 부재**(Noop·shell-only·미구성). 스폰 계속.
    ///   - `Err(_)`       — 제어 채널을 쓰려다 **실패**(CSPRNG/파일 write 오류). ★치명★ — 스폰은
    ///     이 Err 를 만나면 fail-closed 로 중단한다(제어 채널 없이 몰래 도는 에이전트 금지, health 위장 방지).
    ///
    /// `accepts_mcp_config`(ADR-0099): 이 backend 가 mcp-config 를 받아들이는가(= MCP-capable 인가). manager 가
    ///   `backend::accepts_mcp_config(command)` 로 판정해 넘긴다. 데몬 구현이 이 플래그로 **MCP 입구·
    ///   프라이밍 변형·grant·우편 가부를 한꺼번에 가른다**(정합 불변식 = 프라이밍이 **가르치는** 우편 채널
    ///   **=** 그 스폰이 실제로 쓸 수 있는 우편 채널. 못 쓰는 채널을 가르치면 ADR-0099 가 실측한 발신
    ///   freeze 가 재발한다).
    ///   true → mcp-config 기록 + MCP endpoint bits + MCP-only 교육 프라이밍(`send_message` 만 — ADR-0126
    ///   결정 1) + `mail_allowed=false`(데몬이 이 자격증명의 우편 요청을 거절한다 — ADR-0133).
    ///   false → mcp-config **미기록** + CLI-only 프라이밍 + `mail_allowed=true`.
    ///   `engram` CLI 배선(env·PATH)은 **두 갈래 모두** 받는다 — 제어 동사가 전원 개방이기 때문이다
    ///   (ADR-0132 결정 5).
    // ADR-0126
    // ADR-0133
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        accepts_mcp_config: bool,
    ) -> Result<Option<ControlEndpoint>, ProvisionError>;

    /// (AgentId,epoch)의 토큰을 폐기하고 mcp-config 파일을 지운다. 어떤 terminal(kill·크래시·EOF·정상
    /// 종료)에서든 reaper 가 부르므로 누락이 없다. kill_agent 와 spawn 실패 가드도 선제로 부르니 같은
    /// (id,epoch) 에 중복 호출이 온다(remove-if-present 로 흡수). epoch 를 함께 받아 stale terminal 이
    /// 재활성화(새 화신 = 다른 표식)로 새로 붙은 산 토큰을 지우지 못하게 한다(ADR-0007/0084 epoch-guard
    /// 정신 — 판정은 **일치/불일치**다. 표식엔 순서가 없다).
    fn revoke(&self, id: AgentId, epoch: u32);
}

/// 제어 채널을 안 쓰는 경로(headless 테스트·shell-only)용 no-op 구현. provision 은 항상 Ok(None)
/// (정당한 부재 — 실패가 아님), revoke 는 무동작. AgentManager 기본값 — 데몬만 실제
/// `DaemonControlChannel` 을 주입한다.
pub struct NoopControlChannel;

impl ControlChannel for NoopControlChannel {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
        _accepts_mcp_config: bool,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Ok(None)
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

/// 영역별 capability (bool 폭증 금지). 직렬화(프론트 공유, snake_case).
///
/// ★출처 분리(load-bearing)★: 이 합성값의 5영역은 **두 출처**에서 온다 — input/output/control은
/// 물리 채널(transport)이, session/model은 프로그램(backend)이 결정한다. 예전엔 transport가
/// session.resume 까지 하드코딩해(claude·shell 무관 resume=true) shell 백엔드가 부정확했다.
/// 이제 `Capabilities::compose(TransportCaps, BackendCaps)`로만 만들어 출처를 타입으로 강제한다
/// (ADR-0030 capability 매트릭스).
// ADR-0030
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

/// session/model 이 여기 없는 건 의도다 — transport 는 그걸 만들 수 없다(소유권을 타입으로 강제).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransportCaps {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
}

/// 실행 대상 프로그램(claude/shell/codex…)의 능력. input/output/control 이 여기 없는 건 의도다 —
/// backend 는 그걸 만들 수 없다(소유권을 타입으로 강제).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendCaps {
    pub session: SessionCaps,
    pub model: ModelCaps,
}

impl Capabilities {
    /// Capabilities 의 **유일한 정상 생성 경로** — 출처가 섞이지 않게 타입으로 박았다.
    pub fn compose(t: TransportCaps, b: BackendCaps) -> Capabilities {
        Capabilities {
            input: t.input,
            output: t.output,
            control: t.control,
            session: b.session,
            model: b.model,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// 출력이 구조화 스트림(NDJSON 등)이라 터미널 렌더가 아닌 파싱 렌더(RichSlot)가 필요함을 신고(ADR-0044).
    /// 프론트 `defaultRenderMode` 가 이 값 하나로 렌더러를 가른다(true=RichSlot / false=xterm, ADR-0002).
    /// 내용 해석 아님(통로 무정제 불변) — "이 바이트 스트림은 터미널이 아니다"라는 렌더 힌트일 뿐.
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

/// drain 내부 전달용 raw PTY 출력 청크 — 바이너리 그대로 (UTF-8 쪼개짐 방지)
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}

/// 프론트로 나가는 PTY 출력 wire 포맷 — base64 인코딩으로 JSON 호환.
/// ※S12: 이건 **Embedded(Tauri JSON Channel) 전용** 표현. base64는 JSON Channel 제약이며
/// 코어 관심사가 아니다 — ChannelOutputSink가 OutputFrame(raw)을 받아 이걸로 인코딩한다.
///
/// ★epoch★: WS binary frame 헤더([tag][agentId][epoch][seq])와 동형으로 출력 frame 마다
/// 세션 epoch 을 싣는다(OutputFrame.epoch 그대로). 인코딩 시 frame.epoch 을 **버리면**
/// embedded 가 epoch 0 고정으로 흘러, SubscribeAck.current_epoch≥1(resume-fallback) 과
/// 불일치해 ProtocolClient epoch 가드(f.epoch !== st.epoch)가 출력을 전멸시킨다(Stage 3
/// BLOCKER 1). 따라서 frame.epoch 을 반드시 동봉해 WS 경로와 동형화한다.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq: u64,
    pub epoch: u32,
    pub data_b64: String,
}

/// 코어→sink 출력 payload (S15 B5 payload-generic). **빌려서** 전달 — 코어는 wire 를 모른다(ADR-0003).
/// ★ADR-0002 (출력 종류 비가정)★: 출력을 터미널 바이트로 강제하지 않는다 — Bytes/Event 두 갈래로
/// 나눠 sink 가 종류별로 처리(Bytes→tag0 terminal frame, Event→tag1 structured frame, B7)한다.
/// 참조만 담아 Copy 유지(OutputFrame Copy 계약 보존) — Serialize 미부착(core 도메인 타입, ADR-0003).
#[derive(Debug, Clone, Copy)]
pub enum OutputPayload<'a> {
    /// 콘솔 raw 바이트(터미널·tag0 경로). PtyTransport·터미널 모드의 payload.
    Bytes(&'a [u8]),
    /// 구조화 이벤트(tag1 경로 — B7 이 인코딩). backend decoder 가 파싱한 OutputEvent.
    Event(&'a OutputEvent),
}

/// 코어→sink 출력 경계(S15 B5 payload-generic). **payload 를 빌려서** 전달 — Copy(참조만)라 fanout 시
/// 복사 0. agent_id/epoch는 OutputCore가 보유한 불변값을 그대로 싣는다(데몬 frame 헤더용).
#[derive(Debug, Clone, Copy)]
pub struct OutputFrame<'a> {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub payload: OutputPayload<'a>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    /// canonical 표시명 = display_name(override) ?? basename(cwd) — 프론트 트리·라우팅과 같은 문자열
    /// (ADR-0101). 파생할 cwd 세그먼트가 없을 때만 id 앞 8자로 degrade.
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    /// 화신마다 새로 뽑는 난수 표식 — 순서 증분이 아니다(ADR-0163). 프론트 재구독 deps 에는 넣지
    /// 않는다(ADR-0164 결정 8, 옛 서술 S9 §18-a는 낡았다).
    pub epoch: u32,
    pub capabilities: Capabilities,
    /// 이 에이전트가 **편지를 읽는 주체**인가(= 우편 수신자 명단 자격). 세션이 spawn 때 backend 에서
    /// 받아 든 값을 **이 스냅샷에 그대로 실어** 내보낸다.
    ///
    /// ★스냅샷에 싣는 이유(load-bearing — 재조회로 되돌리지 마라)★: 소비자가 목록을 받아 놓고 항목마다
    ///   매니저에 되물으면 ① 자격 확인과 주입 사이에 같은 id 가 다른 세션으로 갈릴 수 있고(TOCTOU)
    ///   ② 두 조회 사이에 reaper 가 세션을 거두면 그 항목이 산 명단에서도 잠듦에서도 떨어져, 파킹돼
    ///   재등장 때 배달되던 편지가 "그런 수신자 없음" 으로 **유실**되며 ③ 세션당 락 획득이 N회 되살아난다
    ///   (`AgentManager::roster` doc — 우편 발송 임계 경로). 한 스냅샷에서 나오면 셋 다 성립하지 않는다.
    /// ★capability 가 아니다★: `capabilities` 는 "무엇을 할 수 있나", 이건 "입력이 무엇으로 해석되나" 다.
    ///   그래서 `Capabilities` 안이 아니라 별도 필드이고, wire 로도 나가지 않는다(데몬 내부 판정 축).
    pub reads_messages: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("agent not found: {0}")]
    NotFound(AgentId),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("write failed: {0}")]
    WriteFailed(String),
    #[error("cwd outside workspace")]
    CwdDenied,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// transport가 해당 동작을 지원하지 않음(ApiTransport 껍데기 등). 동사별 미지원 신호.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// ★명부 총량 상한에 걸려 **새 에이전트 등록을 거부**했다(폭주 백스톱 — `AgentManager::MAX_ROSTER_SIZE`)★.
    ///
    /// ★전용 변형인 이유★: 호출부가 이걸 이름 공간 소진(`Unsupported`)과 구분해야 한다 — 사용자가 할 일이
    ///   정반대다(이름을 바꿔 다시 시도 vs 에이전트를 지워 자리를 비우기). 문자열 매칭으로 가르는 것은
    ///   문구가 바뀌는 순간 조용히 깨진다.
    /// ★기존 에이전트에는 걸리지 않는다★: 상한은 **신규 등록**만 본다. 이미 상한을 넘은 명부의 복원·재spawn
    ///   (같은 id 의 화신 교체)은 그대로 통과한다 — 백스톱이 기존 팀을 인질로 잡으면 복구가 불가능해진다.
    #[error("roster is full: {current} agents (ceiling {limit}) — refusing to register another")]
    RosterFull { current: usize, limit: usize },
    // ★중복 spawn 요청을 여기 오류로 되돌리지 마라★: "이미 떠 있다 / 이미 뜨는 중이다" 는 실패가 아니라
    //   **할 일이 없는 요청**이라 `Ok(manager::SpawnOutcome::Moot)` 로 답한다. 오류로 두던 시절엔 소비자
    //   마다 "이 오류는 진짜 실패가 아니다" 목록을 들어야 했고, 그 목록을 세 번 손보는 동안 세 번 다 하나씩
    //   빠져 산 에이전트에 실패 표시가 찍혔다(ADR-0172).
}

/// 구독 replay 분기 결과(코어 중립 — 데몬이 protocol::SubscribeAction 으로 매핑).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayKind {
    /// 처음(oldest)부터 전체 replay — 신규 구독 또는 epoch 불일치.
    FromOldest,
    /// after_seq 가 ring oldest 보다 과거 → oldest 부터(앞부분 손실).
    Truncated,
    /// after_seq+1 부터 무손실 이어받기(tail 만).
    Resumed,
}

/// subscribe_from 결과 메타(데몬이 SubscribeAck 구성에 사용).
#[derive(Debug, Clone, Copy)]
pub struct SubscribeOutcome {
    pub kind: ReplayKind,
    pub sink_id: SinkId,
    pub oldest_seq: u64,
    pub latest_seq: u64,
    /// 실제 처음 전송한 chunk 의 seq. 보낼 게 없으면 "다음 live seq" 추정치.
    pub replay_from: u64,
    /// 실제 전송한 chunk 수(0 가능).
    pub replayed: usize,
}

/// 입력 write 의 배달-경계 계측 산출물(ADR-0088 Stage 0).
///
/// ★왜 존재하나★: `write_input`/`write_stdin` 이 `Ok(())` 만 돌려주면 "전송 실패로 안 꽂힘" 과
///   "다 꽂혔는데 모델이 무시" 를 구별할 증거가 없다. 배달 정확성 하네스(ADR-0088)가 이 둘을
///   가르려면 write 경계에서 **완결성 신호**(전량 수용 vs 실패)와 이 유저 턴의 replay-dedup 키
///   (`msg_uuid`)를 관측 가능하게 올려야 한다. 이 값이 그 산출물이다(성공 경로에서만 반환).
///
/// ★완결성 신호 = Ok-vs-Err 이지 바이트 비교가 아니다(중요)★: transport 의 `send_input` 은
///   `write_all`(+`flush`)로 쓴다 — `write_all` 은 요청 바이트를 **전부** 쓰거나 `Err` 를 낸다
///   (부분 write 를 `Ok` 로 숨기지 않는다, std 계약). 따라서 "전량 수용됐나"의 유일한 증거는
///   `Ok(WriteOutcome)` **자체**(vs `Err`)다 — 아래 두 바이트 필드의 비교가 아니다. 진짜 written
///   바이트 수는 transport 밖으로 스레드되지 않으므로(write_all 계약상 불필요), `bytes_written` 은
///   독립 계측값이 아니라 `bytes_requested` 를 **구성상 그대로 복사**한 값이다(short-write 탐지 불가 —
///   비교하면 항상 같다, 동어반복). 이 필드가 있는 이유는 관측 레코드의 자기설명(로그·forensic 에서
///   "이만큼을 write 요청했고 write_all 이 Ok 였다"는 by-construction 항등)일 뿐, 완결성 판정 레버가
///   아니다. 완결성은 `Ok` 를 봐야 한다.
///
/// ★바이트 단위·계층★: 여기 두 값은 **호출자가 세션 경계에 넘긴 논리 메시지의 바이트 길이**
///   (`bytes.len()` — encoder 감싸기 **전**, char 수 아님)다. encoder 가 텍스트를 감싸면(json 모드)
///   실제 wire 바이트는 이보다 크지만, 그 encoded wire 카운트는 여기서 재지 않는다(이 계층의 논리
///   단위가 아님). daemon 레이어의 `DeliveryObservation` 도 같은 "논리 메시지 바이트" 의미를 쓴다
///   (거기선 그 논리 메시지 = `wrap_message` 로 만든 봉투 문자열의 바이트).
#[derive(Debug, Clone, Copy)]
pub struct WriteOutcome {
    pub bytes_requested: usize,
    pub bytes_written: usize,
    /// 이 유저 턴의 메시지 uuid(replay-dedup 키, session.write_input 이 생성 — LOAD-BEARING).
    /// 배달 하네스가 ingress 의 논리 msg_id 와 이 값을 상관시켜 "claude 가 이 턴을 replay 했나"(=
    /// 실제 파싱했나)를 판정한다(ADR-0088). 값·의미는 여기서 바꾸지 않는다 — 노출만 한다.
    pub msg_uuid: uuid::Uuid,
    /// ★write 가 실제로 착지한 incarnation 의 epoch(ADR-0088 Stage 1)★. 이 write 를 수행한 세션의
    /// `self.epoch` 를 by-construction 으로 실은 값이다(bytes_written 과 같은 성격 — 독립 측정이 아니라
    /// write 를 집행한 세션이 자기 epoch 을 그대로 채운다). ★왜 필요한가★: 제어 채널 배달 관측 레코드가
    /// **자기충족(record-self-sufficient)** 이려면 "이 메시지가 어느 incarnation 에 꽂혔나" 를 레코드
    /// 안에서 직접 답할 수 있어야 한다 — resolve 시점 스냅샷 epoch 이 아니라 **write 가 실제로 착지한**
    /// 세션의 epoch 을 담아야 mid-flight epoch race(resolve↔write 사이 재시작) 를 레코드만으로 단정할 수
    /// 있다(그 race 는 ADR-0086 §F5 가 design-accepted 로 표시 — 메일은 논리 에이전트를 향하므로 새
    /// incarnation 착지가 올바른 동작이다). 값은 write 를 집행한 세션에서만 채워지므로(성공 경로 한정)
    /// resolve-time 과 어긋날 수 있고, 바로 그 어긋남을 관측하려고 존재한다.
    pub epoch: u32,
}

/// OutputSink 전송 실패 신호 — drain이 감지 시 해당 구독자 제거 트리거
#[derive(Debug)]
pub struct SinkError;

/// PTY 출력 전달 추상화 — Tauri 의존 없이 headless 테스트 가능하게 격리.
/// ※S12: wire 인코딩은 구현체가 소유한다(ChannelOutputSink=base64 PtyEvent / 데몬 프레임 sink=binary
/// frame) → 코어 transport-agnostic.
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}

/// 에이전트 상태 변경 알림 추상화 — pty/가 AppHandle 없이 상위 층에 통보
pub trait StatusSink: Send + Sync + 'static {
    /// epoch 동봉(S9 §18-d): 프론트가 재spawn 후 옛 세션의 지연된 terminal 알림을
    /// epoch 불일치로 버릴 수 있게 한다(stale Killed 방어, fable C-1/Mn-1).
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
    /// 복원 시도 결과 통지(S9 §18-d). 기본 no-op — 복원을 안 쓰는 sink는 구현 불필요.
    fn restore_result(&self, _report: crate::agent::profile::RestoreReport) {}
    /// 이 화신이 방금 한 턴을 끝냈다(ADR-0113 — 턴 관측의 push 출구). 기본 no-op.
    ///
    /// ★계약 = 논블록·비재진입(ADR-0006 콜백 규율 — 절대 위반 금지)★: 구현이 할 수 있는 일은 논블록
    ///   통지(채널 send) 정도다. IO·주입·재진입 호출을 하면 그 작업이 끝날 때까지 이 콜백을 부른
    ///   스레드가 멈춘다.
    /// ★그 스레드는 **오늘은** 출력 pump 다 — 그러나 구조가 보장하지 않는다(load-bearing)★: `emit` 의
    ///   호출자는 둘이고(출력 pump · 입력 에코를 낸 주입 스레드 — turn.rs 헤더), 이 훅은 그중 종료 신호를
    ///   낸 쪽에서 불린다. 지금 pump 로 고정되는 이유는 **입력 에코가 진행 신호로 분류되기 때문**일 뿐이다
    ///   (backend 분류자). 종료 신호를 내는 비-pump emit 경로가 새로 생기면 이 계약의 무게가 그 스레드로
    ///   옮겨간다 — 그때 배달 작업을 발신 스레드에 얹지 않으려면 여기가 여전히 논블록이어야 한다.
    /// ★전이 여부를 가리지 않고 종료 신호마다 나간다★: 소비자가 "직전이 턴 중이었을 때만" 으로 좁히려면
    ///   그 판정을 자기 쪽에서 해야 한다. 여기서 좁히면 어시스턴트 이벤트 없이 곧장 끝나는 턴에서 통지가
    ///   빠지는데, 잉여 통지는 대개 무해한 반면 누락은 소비자를 영구 대기시킨다.
    // ADR-0113
    fn turn_ended(&self, _id: AgentId, _epoch: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Capabilities::compose — 출처가 올바른 영역으로 합쳐지는지(소유권 합성 검증) ──
    #[test]
    fn compose_merges_each_source_into_its_region() {
        let t = TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                structured: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: false,
            },
        };
        let b = BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        };

        let caps = Capabilities::compose(t, b);

        assert!(
            caps.control.resize,
            "resize 는 transport 가 결정 → 합성에 보존"
        );
        assert!(
            caps.session.resume,
            "resume 은 backend 가 결정 → 합성에 보존"
        );
        assert!(caps.input.raw);
        assert!(caps.output.terminal_bytes);
        assert!(caps.session.cwd_env);
        assert!(!caps.model.select);
    }
}
