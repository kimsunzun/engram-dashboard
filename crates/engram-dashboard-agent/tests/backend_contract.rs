//! 백엔드 계약 시험대 — 백엔드마다 **같은 질문표**를 묻고, 답을 실측으로 채운다(S21 TRD §3).
//!
//! 이 파일이 소유하는 것은 질문표 하나다. 백엔드가 셋째로 늘면 [`backend_table`] 에 **행이 하나 늘고**
//! 질문 함수들은 그대로다 — 파일이 느는 구조가 아니다.
//!
//! **레인이 둘이다.**
//! - 비-ignore 1건([`declaration_table_is_filled_for_every_backend`]) — 프로세스를 하나도 안 띄우고
//!   선언 열만 대조한다. 기본 회귀·CI 에서 그대로 돈다.
//! - 나머지 전부 `#[ignore]` — 실 CLI 를 띄운다. `-- --ignored` 로 부를 때만 돈다.
//!
//! ★비-ignore 항목이 있는 이유 = `#[ignore]` 의 대가★: `#[ignore]` 스위트는 통째로 증발해도 초록이다.
//!   컴파일과 표의 존재를 지키는 항목이 하나는 남아 있어야 그 침묵이 안 생긴다.
//!
//! ★`--ignored` 레인 안에서 바이너리 부재는 skip 이 아니라 실패다★: 이 레인을 부른 사람은 "재겠다" 고
//!   말한 것이라 조용한 초록을 돌려주면 안 된다. 게다가 Windows 에서 CLI 는 `cmd.exe /c <prog>` 로
//!   감싸져 뜨므로 **바이너리가 없어도 `cmd.exe` 는 반드시 성공적으로 뜬다** — "스폰이 됐나" 로는 유무를
//!   영영 못 가른다. 그래서 [`assert_program_present`] 가 실 스폰보다 먼저 선다.
//!
//! **§3-5 게이트** — 아래가 빨개지면 Phase 1 배선을 시작하지 않는다: **Q1 · Q4 · Q5 · Q8 · Q10**.
//! Q3 · Q6 · Q9 는 게이트가 아니라 관측이다(어느 쪽으로 나오든 배선의 *모양* 이 아니라 인자 하나·기록이
//! 갈린다). Q7(사람 눈 — 우리 xterm 에서 보기에 나은가)은 **이 시험대가 덮지 않는다**: 바이트에
//! alt-screen 시퀀스가 있나(Q6)는 잴 수 있어도 "그래서 화면이 나은가" 는 여기서 못 잰다.
//!
//! ★TRD 가 몰랐던 사실 — 첫 방문 폴더에는 **모달**이 먼저 뜬다★(실측 codex-cli 0.153.4): 폴더 신뢰
//!   확인이 컴포저 위에 겹쳐 뜨고, 그 동안 키 입력은 컴포저가 아니라 모달로 간다. 그래서 컴포저·cwd 를
//!   묻는 질문(Q4·Q5·Q6·**Q8**)은 전부 [`PtySession::pass_startup_modal`] 로 그것을 먼저 지나간다.
//!   ★Q8 이 그 명단에 든 것이 이 파일의 가장 비싼 교훈이다★ — 모달이 프로세스를 붙들고 있는 동안은
//!   "5초 뒤에도 Running" 이 **거짓일 수가 없어서**, 모달을 안 지나던 옛 Q8 은 cwd 정책을 물은 적이 없다.
//!   초록이던 그 칸이 사실은 빈 칸이었다. **우리 스폰 입구는 임의 폴더를 받으므로 이건 시험대만의 사정이
//!   아니다** — 처음 보는 cwd 로 뜬 codex 탭은 사람이 모달을 지나기 전까지 입력을 안 받는다.
//!
//! ★**실 사용자 홈을 건드리지 않는다**★: 모달을 지나면 그 CLI 는 폴더 신뢰를 **자기 설정 파일에 적는다**.
//!   그대로 두면 시험대가 돌 때마다 개발자의 실 `~/.codex/config.toml` 에 지워진 경로의 죽은 항목이
//!   쌓인다(실제로 쌓였다 — 그것이 [`LiveProbe::home_redirect`] 가 생긴 계기다). 그래서 이 시험대의 모든
//!   실 스폰은 그 CLI 의 상태 디렉터리를 **항목마다 새로 파는 임시 홈**으로 돌리고, 인증에 필요한 파일만
//!   실 홈에서 복사해 온다. 임시 홈은 `CARGO_TARGET_TMPDIR` 아래 둔다 — ★시스템 temp 아래는 안 된다★:
//!   codex 0.153.4 는 temp 아래를 홈으로 받으면 sandbox 헬퍼 생성을 거부하고 경고를 찍는다(실측).
//!
//! 실 자식 프로세스를 띄우므로 `-- --test-threads=4` 로 돈다(TRD §7 0-2 · CLAUDE.md 「빌드·검증 명령」).
//!   ★사내 보안 에이전트 DLL 이 프로세스 생성을 후킹하는 PC 에서는 1 로 낮춘다★ — 같은 문서의 「그래도
//!   죽으면 2로 낮춘다」 조항과 같은 사유이고, 이 스위트는 한 항목이 실 CLI 를 여러 번 띄운다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{
    AgentBackend, ClaudeBackend, CodexBackend, InputEncoder, SUBMIT_PACING,
};
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::profile::{AgentCommand, ClaudeOutputFormat};
use engram_dashboard_agent::transport::pty::PtyTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

// ── 질문표 ───────────────────────────────────────────────────────────────────

/// 실행 없이 대조하는 열 — 백엔드가 **스스로 신고하는** 값 전부. 새 백엔드 행은 이 열 열 칸을 다
/// 적어야 컴파일된다(그것이 "표가 채워져 있는가" 의 실물).
struct Declared {
    needs_session: bool,
    supports_control_channel: bool,
    accepts_mcp_config: bool,
    reads_messages: bool,
    session_resume: bool,
    session_snapshot: bool,
    session_cwd_env: bool,
    model_select: bool,
    model_temperature: bool,
    model_max_tokens: bool,
}

/// 실 프로세스로 묻는 열. `None` 인 행은 `--ignored` 레인에서 아예 안 뜬다.
struct LiveProbe {
    /// PATH 로 해석되는 bare 실행파일 이름.
    program: &'static str,
    /// 대화형 TUI 를 띄우는 인자. ★샌드박스·승인 정책을 여기서 못 박는다★ — 시험대가 띄우는
    /// 에이전트는 아무것도 바꿀 수 없어야 하고, 동시에 사람 승인을 기다리며 멈춰서도 안 된다.
    interactive_args: &'static [&'static str],
    /// Q6 — alt-screen 을 끄는 인자. `None` = 그런 인자가 없는 백엔드 → **Q6 은 그 행을 건너뛴다**.
    ///
    /// ★왜 `Option` 인가★: 필수 `&'static str` 로 두면 그 인자가 없는 셋째 백엔드가 행을 채우려고
    ///   **자리채움 문자열**을 적어야 하고, Q6 은 그것을 진짜 인자로 알고 스폰한다. "셋째 백엔드 = 행 하나"
    ///   라는 이 파일의 계약이 그 자리에서 깨진다.
    no_alt_screen_arg: Option<&'static str>,
    /// Q3 — 파이프 stdin 을 거부할 때 그 CLI 가 내는 **말**. 종료코드만으로는 "stdin 이 tty 가 아니라서"
    /// 죽었는지 다른 이유로 죽었는지 못 가른다(우리는 세 fd 를 전부 파이프로 준다). `None` = 그 문구를
    /// 모르는 백엔드 → 종료코드만 본다.
    stdin_refusal_marker: Option<&'static str>,
    /// 이 CLI 의 **사용자별 상태 디렉터리**를 옮기는 길. `None` = 그런 길이 없는 백엔드 → 시험대가 실
    /// 사용자 설정을 건드리게 되므로, 그 백엔드 행을 들일 때 복원 책임을 따로 져야 한다.
    home_redirect: Option<HomeRedirect>,
    /// Q9 — 호출자가 세션 id 를 정하는 플래그 이름. `None` = "그런 플래그가 없다" 는 주장이고,
    /// Q9 가 `--help` 텍스트로 [`SESSION_ID_CANDIDATES`] 의 부재를 확인한다.
    session_id_flag: Option<&'static str>,
    /// Q4 — 컴포저에 넣어 에코를 확인할 한 줄 본문. ★제출되면 모델이 실제로 답한다★ — 무해하고
    /// 짧은 것만 둔다.
    submit_body: &'static str,
    /// Q5 — LF 로 이어 붙일 조각들. 조각이 전부 에코되면 여러 줄이 통째로 들어간 것이다.
    multiline_parts: &'static [&'static str],
    /// 첫 방문 폴더에서 이 CLI 가 띄우는 **모달**의 화면 표식. 모달이 떠 있는 동안 키 입력은 컴포저가
    /// 아니라 모달로 가므로, 컴포저를 묻는 질문(Q4·Q5·Q6)은 그것을 지나야 자기 질문을 물을 수 있다.
    /// `None` = 그런 모달이 없는 백엔드.
    startup_modal_marker: Option<&'static str>,
    /// 컴포저가 **비어 있을 때** 그 자리에 그려지는 안내 문구.
    ///
    /// ★왜 필요한가★: 제출은 컴포저를 비운다 — 그러니 본문을 넣은 뒤 이 문구가 **다시 그려지면** 그
    ///   입력은 제출된 것이다. 이 신호가 없으면 Q5 는 "본문 아닌 글자가 왔나" 로 판정할 수밖에 없는데,
    ///   TUI 는 아무 입력 없이도 배너·모델명·사용량 안내를 다시 그려서 그 글자가 **응답인지 제 화면인지
    ///   구분되지 않는다**(실측 — 그 잡음이 게이트를 헛발동시켰다).
    /// `None` = 그 신호가 없는 백엔드 → Q5 는 화면 정적 여부만 본다.
    composer_idle_marker: Option<&'static str>,
    /// Q6 — `--no-alt-screen` 이 **PTY 바이트를 바꾸는가**. Phase 0 이 실측으로 채우는 칸이고, 테스트는
    /// 관측이 이 선언과 같은지를 잰다.
    ///
    /// ★왜 「달라야 한다」로 단언하지 않나★: Q6 은 게이트가 아니라 관측이라 어느 쪽 값이든 정답이다
    ///   (§3-5 에 이 행이 없는 이유). 값을 여기 적어 두면, 그 값이 바뀌는 날 이 항목이 빨개져 §4-11 의
    ///   권고를 다시 읽으라고 말해 준다 — 한쪽으로 박아 두면 그건 결정을 테스트에 숨기는 것이다.
    no_alt_screen_changes_bytes: bool,
}

/// CLI 의 상태 디렉터리를 임시 홈으로 돌리는 법. 이것이 있어야 시험대가 **개발자의 실 설정을 안 건드린다**.
struct HomeRedirect {
    /// 상태 디렉터리를 가리키는 환경변수 이름(codex = `CODEX_HOME`, 실측 0.153.4 — 임시 홈으로 돌리면
    /// `codex mcp list` 가 실 홈의 서버를 못 본다).
    env: &'static str,
    /// 그 변수가 안 걸렸을 때의 기본 위치(사용자 홈 기준 상대 경로). 씨앗 파일을 여기서 퍼 온다.
    default_subdir: &'static str,
    /// 임시 홈에 **복사해 넣어야 그 CLI 가 여전히 로그인 상태인** 파일들. 없으면 TUI 가 컴포저 대신
    /// 로그인 화면을 띄워 Q1 부터 무너진다(실측: 빈 홈 = `Not logged in`).
    ///
    /// ★복사본은 자격증명이다★ — 임시 홈은 [`Scratch`] 가 `Drop` 에서 지운다. 그 보증이 이 필드의 전제다.
    seed_files: &'static [&'static str],
    /// 임시 홈에 **새로 적어 넣을** 파일 — `(파일명, 내용)`. 복사보다 나중에 적용된다.
    ///
    /// ★왜 필요한가 — 실측 2026-09-07★: 빈 홈으로 codex 0.153.4 를 띄우면 폴더 신뢰 모달 **뒤에 모달이 하나
    ///   더** 온다(`Set up the Codex agent sandbox …`). 그 둘째 모달의 기본 선택은 `1. Set up default sandbox
    ///   (requires Administrator permissions)` 라 ★자동 테스트가 CR 로 지나가서는 안 되는 화면★이다 —
    ///   지나가면 관리자 권한 상승을 부른다. 그리고 그 모달이 떠 있는 동안 컴포저는 키를 안 받으므로,
    ///   Q4·Q5 는 아무 에코도 못 받고 죽는다(실제로 그렇게 죽었고, 그 시체가 이 필드의 근거다).
    /// ★그래서 「실 홈이 이미 갖고 있는 답」을 그대로 적어 준다★: 개발자의 실 `config.toml` 에는
    ///   `[windows] sandbox = "unelevated"` 가 이미 들어 있어 그 모달이 안 뜬다. 운영 스폰이 만나는 것도
    ///   그 홈이므로, 이걸 적는 것은 관측을 꾸미는 것이 아니라 **운영과 같은 조건으로 맞추는 것**이다.
    ///   ★단 폴더 신뢰 항목(`[projects.*]`)은 적지 않는다★ — 그 모달은 지나가는 것이 Q4·Q5·Q8 의 관측 대상이다.
    seed_writes: &'static [(&'static str, &'static str)],
}

struct BackendRow {
    name: &'static str,
    backend: &'static dyn AgentBackend,
    /// 선언을 물을 때 넘기는 명령 표본.
    sample: AgentCommand,
    declared: Declared,
    probe: Option<LiveProbe>,
}

/// Q9 가 `--help` 에서 부재를 확인하는 이름들. 하나라도 나오면 그 백엔드는 호출자가 세션 id 를 정할
/// 수 있다는 뜻이라 [`LiveProbe::session_id_flag`] 가 `None` 인 것이 거짓이 된다.
const SESSION_ID_CANDIDATES: &[&str] = &["--session-id", "--session", "--resume"];

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static CODEX_BACKEND: CodexBackend = CodexBackend;

fn backend_table() -> Vec<BackendRow> {
    vec![
        BackendRow {
            name: "claude",
            backend: &CLAUDE_BACKEND,
            sample: AgentCommand::Claude {
                extra_args: vec![],
                output_format: ClaudeOutputFormat::Terminal,
            },
            declared: Declared {
                needs_session: true,
                supports_control_channel: true,
                accepts_mcp_config: true,
                reads_messages: true,
                session_resume: true,
                session_snapshot: false,
                session_cwd_env: true,
                model_select: false,
                model_temperature: false,
                model_max_tokens: false,
            },
            // 실 claude 를 띄우지 않는다: 이 열은 이미 실측으로 채워져 있고, 실 claude 의존 테스트가
            // CI 에서 **fn 이름으로 `--skip`** 되는 미결이 바로 그 형태다(TRD §3-2). 시험대는 그 목록을
            // 늘리지 않는다.
            probe: None,
        },
        BackendRow {
            name: "codex",
            backend: &CODEX_BACKEND,
            sample: AgentCommand::Codex { extra_args: vec![] },
            declared: Declared {
                // ★실측이 도장 찍은 값이다★ — 호출자가 세션 id 를 정할 수 없고(`session_id_flag: None`
                //   이 그 짝), 턴을 관측할 수 없어 바쁜 때를 못 가리므로 수신자 명단에서 뺀다. 사유의
                //   정본은 `backend/codex/`.
                needs_session: false,
                supports_control_channel: false,
                accepts_mcp_config: false,
                reads_messages: false,
                session_resume: false,
                session_snapshot: false,
                session_cwd_env: true,
                model_select: false,
                model_temperature: false,
                model_max_tokens: false,
            },
            probe: Some(LiveProbe {
                program: "codex",
                // `-s read-only` = 모델이 내는 명령을 읽기 전용 샌드박스로만 실행 ·
                // `-a never` = 승인을 사람에게 묻지 않는다.
                interactive_args: &["-s", "read-only", "-a", "never"],
                no_alt_screen_arg: Some("--no-alt-screen"),
                // 실측 M1(TRD §2) 의 그 문장 그대로. Q3 은 이 말이 나와야 "stdin 이 tty 가 아니라서" 를
                // 단언할 수 있다.
                stdin_refusal_marker: Some("stdin is not a terminal"),
                home_redirect: Some(HomeRedirect {
                    env: "CODEX_HOME",
                    default_subdir: ".codex",
                    // 인증 하나면 로그인 상태가 따라온다(실측: 이 파일만 복사한 빈 홈에서
                    // `codex login status` = `Logged in using ChatGPT`). `version.json` 은 자동 업데이트
                    // 확인 기록이라, 같이 옮겨 두면 항목마다 새 홈을 파도 업데이트 검사가 다시 안 돈다.
                    seed_files: &["auth.json", "version.json"],
                    // 실 홈의 `[windows] sandbox` 값 그대로. 이게 없으면 신뢰 모달 뒤에 관리자 권한을 묻는
                    // sandbox 설치 모달이 따라온다(실측 — 위 필드 주석).
                    seed_writes: &[("config.toml", "[windows]\nsandbox = \"unelevated\"\n")],
                }),
                session_id_flag: None,
                submit_body: "say OK",
                multiline_parts: &["hi", "there"],
                // 폴더 신뢰 확인 모달(실측 codex-cli 0.153.4). 줄바꿈에 걸리지 않게 짧게 집는다.
                startup_modal_marker: Some("Do you trust"),
                composer_idle_marker: Some("Ask Codex to do anything"),
                // 실측 0.153.4 — 두 모드의 DEC private mode 집합이 **완전히 같고** 어느 쪽도 alt-screen
                // 진입 시퀀스를 안 낸다. 즉 이 버전의 TUI 는 플래그 없이도 inline 이다.
                no_alt_screen_changes_bytes: false,
            }),
        },
    ]
}

// ── 선언 항목 (실행 없음 — CI 에서 돈다) ─────────────────────────────────────

#[test]
fn declaration_table_is_filled_for_every_backend() {
    let table = backend_table();
    assert!(
        !table.is_empty(),
        "질문표가 비었다 — 시험대가 아무것도 안 재고 있다"
    );

    for row in &table {
        let b = row.backend;
        let d = &row.declared;
        let name = row.name;

        assert_eq!(b.needs_session(), d.needs_session, "{name}: needs_session");
        assert_eq!(
            b.supports_control_channel(),
            d.supports_control_channel,
            "{name}: supports_control_channel"
        );
        assert_eq!(
            b.accepts_mcp_config(),
            d.accepts_mcp_config,
            "{name}: accepts_mcp_config"
        );
        assert_eq!(
            b.reads_messages(),
            d.reads_messages,
            "{name}: reads_messages"
        );

        let caps = b.capabilities(&row.sample);
        assert_eq!(
            caps.session.resume, d.session_resume,
            "{name}: session.resume"
        );
        assert_eq!(
            caps.session.snapshot, d.session_snapshot,
            "{name}: session.snapshot"
        );
        assert_eq!(
            caps.session.cwd_env, d.session_cwd_env,
            "{name}: session.cwd_env"
        );
        assert_eq!(caps.model.select, d.model_select, "{name}: model.select");
        assert_eq!(
            caps.model.temperature, d.model_temperature,
            "{name}: model.temperature"
        );
        assert_eq!(
            caps.model.max_tokens, d.model_max_tokens,
            "{name}: model.max_tokens"
        );
    }

    // ★`#[ignore]` 레인이 공회전하지 않는다는 것도 여기서 지킨다★: 실 프로브 행이 하나도 없으면
    //   Q1~Q10 은 빈 순회로 전부 초록이 된다. 그 침묵을 이 항목이 잡는다.
    assert!(
        table.iter().any(|r| r.probe.is_some()),
        "실 프로브 행이 하나도 없다 — Q1~Q10 이 빈 순회로 조용히 통과한다"
    );
}

// ── 공용 하네스 ──────────────────────────────────────────────────────────────

const FIRST_BYTES_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE_QUIET: Duration = Duration::from_millis(1500);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(25);
/// Q2 · Q8 — "첫 출력 뒤 이만큼은 안 죽는다".
const ALIVE_WINDOW: Duration = Duration::from_secs(5);
/// Q4 — 제출 뒤 모델 응답이 시작되기를 기다리는 상한.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(45);
/// Q4 · Q5 — CR 없이 본문만 넣고 "이미 응답이 시작됐나" 를 보는 창.
const PRE_SUBMIT_WINDOW: Duration = Duration::from_secs(6);
/// Q6 — 두 모드의 초기 출력을 모으는 창.
const CAPTURE_WINDOW: Duration = Duration::from_secs(8);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);
/// 시작 모달이 화면에 나타나기를 기다리는 상한.
const MODAL_TIMEOUT: Duration = Duration::from_secs(20);
/// 모달을 지난 뒤 컴포저가 그려지기를 기다리는 상한.
const COMPOSER_TIMEOUT: Duration = Duration::from_secs(25);
/// Q4 · Q5 — 넣은 본문이 컴포저에 에코되기를 기다리는 상한. ★고정 sleep 이 아니라 재시도인 이유★:
/// 첫 프레임 한 번이 느리면 고정 대기는 그 자리에서 오판한다(나머지 대기가 전부 [`wait_until`] 인 것과
/// 같은 사유). 단 Q4 는 그 뒤에도 [`SUBMIT_PACING`] 만큼의 간격을 반드시 채운다 — 그 간격이 재는 계약이다.
const ECHO_TIMEOUT: Duration = Duration::from_secs(10);
/// [`OutputCore::join_pump`] 에 주는 상한 — ADR-0001 의 그 5초.
const JOIN_PUMP_TIMEOUT: Duration = Duration::from_secs(5);
/// 파이프 reader 스레드 회수 상한. ★무한 join 금지★ — 직속 자식이 끝나도 **손자가 파이프 쓰기단을 쥐고
/// 있으면** `read_to_end` 는 영영 안 끝난다(shim 사슬은 늘 손자가 있다).
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

fn live_rows() -> Vec<BackendRow> {
    let rows: Vec<BackendRow> = backend_table()
        .into_iter()
        .filter(|r| r.probe.is_some())
        .collect();
    assert!(
        !rows.is_empty(),
        "실 프로브 행이 없다 — 이 레인은 아무것도 재지 않았다"
    );
    rows
}

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    output: Arc<Mutex<Vec<u8>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn snapshot(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    fn tail_from(&self, from: usize) -> Vec<u8> {
        let buf = self.output.lock().unwrap();
        if from >= buf.len() {
            Vec::new()
        } else {
            buf[from..].to_vec()
        }
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

/// `engram_dashboard_agent::backend::console_command`(`src/backend/mod.rs:34-48`) 와 **같은 래핑**.
/// 그 함수는 `pub(crate)` 라 통합 테스트에서 못 부르므로 여기서 다시 만든다.
///
/// ★알려진 미결 — 이건 손으로 복제한 것이고 어긋남을 잡는 게이트가 없다★: 두 쪽이 갈리면 이 시험대는
///   운영이 실제로 띄우는 것과 **다른 것**을 재게 되는데, 오늘 두 본문은 바이트로 같지만 그 사실을 지키는
///   것은 사람의 눈뿐이다. 제대로 된 해법은 이 crate 에 이미 선례가 있는 `test-harness` feature
///   (ADR-0088 / ADR-0012) 뒤로 `console_command` 를 노출해 **부르는 것**이다 — 이 시험대를 고친 작업은
///   "이 파일 하나만 고친다" 는 범위였고 그 seam 은 `src/backend/mod.rs` 를 건드려야 열리므로 남겨 둔다.
fn console_wrapped(program: &str, args: &[&str]) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let mut wrapped = vec!["/c".to_string(), program.to_string()];
        wrapped.extend(args.iter().map(|a| a.to_string()));
        ("cmd.exe".to_string(), wrapped)
    }
    #[cfg(not(windows))]
    {
        (
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
        )
    }
}

#[derive(Debug)]
struct Captured {
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

/// 파이프로만 이야기하는 자식 실행. **stdin 은 언제나 파이프**다 — Q3 이 재는 조건이 그것이고, 나머지
/// 호출도 콘솔을 물려받아 사람 입력을 기다리는 일이 없어야 한다. `timeout` 을 넘기면 트리째 죽이고
/// `timed_out` 을 세운다(반환은 언제나 온다).
fn run_capture(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &Path,
    timeout: Duration,
) -> Captured {
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("{program} spawn 실패: {e}"));

    drop(child.stdin.take());

    // ★파이프를 스레드로 빨아낸다★: `--help` 처럼 4KB 를 넘기는 출력은 파이프 버퍼가 차면 자식이
    //   write 에서 멈춰 종료 폴링이 영영 안 끝난다.
    //
    // ★그 스레드를 `join()` 으로 회수하지 않는다★: 직속 자식이 끝나도 **파이프 쓰기단을 물려받은 손자**가
    //   살아 있으면 `read_to_end` 는 EOF 를 영영 못 본다 — shim 사슬(`cmd.exe → node → codex.exe`)에서는
    //   늘 손자가 있다. 그래서 결과를 채널로 받아 [`READER_JOIN_TIMEOUT`] 만 기다리고, 안 오면 그 스레드는
    //   두고 간다(테스트 프로세스가 끝날 때 함께 사라진다).
    let out_reader = child.stdout.take().map(|s| drain_pipe(s));
    let err_reader = child.stderr.take().map(|s| drain_pipe(s));

    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    kill_tree(pid);
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait 실패: {e}"),
        }
    }

    let stdout = out_reader
        .map(|rx| rx.recv_timeout(READER_JOIN_TIMEOUT).unwrap_or_default())
        .unwrap_or_default();
    let stderr = err_reader
        .map(|rx| rx.recv_timeout(READER_JOIN_TIMEOUT).unwrap_or_default())
        .unwrap_or_default();

    Captured {
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}

/// 파이프 한쪽을 스레드로 빨아내고 결과를 채널로 준다. 회수 상한은 부르는 쪽이 진다.
fn drain_pipe<R: std::io::Read + Send + 'static>(mut src: R) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = src.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// shim 사슬은 직속 자식보다 깊으므로 PID 하나만 죽이면 손자가 남는다.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// ★바이너리 부재 = 이 레인의 실패★. `cmd.exe /c <prog>` 래핑 때문에 스폰 성공은 유무의 증거가 되지
/// 못하므로, 스폰보다 먼저 PATH 해석 자체를 묻는다. 해석된 경로는 그대로 증거로 찍는다 — shim 사슬이
/// 어디서 시작하는지가 Q10 의 전제다.
fn assert_program_present(program: &str) {
    #[cfg(windows)]
    let (finder, args) = (
        "cmd.exe",
        vec!["/c".to_string(), "where".to_string(), program.to_string()],
    );
    #[cfg(not(windows))]
    let (finder, args) = (
        "sh",
        vec!["-c".to_string(), format!("command -v {program}")],
    );

    let found = run_capture(finder, &args, &[], &std::env::temp_dir(), PROCESS_TIMEOUT);
    assert_eq!(
        found.exit_code,
        Some(0),
        "`{program}` 이 PATH 에 없다 — 이 레인은 재겠다고 부른 레인이라 조용히 넘기지 않는다: {found:?}"
    );
    assert!(
        !found.stdout.trim().is_empty(),
        "`{program}` 해석 결과가 비었다: {found:?}"
    );
    println!(
        "[presence] {program} -> {}",
        found.stdout.trim().replace(['\r', '\n'], " | ")
    );
}

/// 임시 폴더 한 칸. ★`Drop` 이 지운다★ — 단언이 패닉으로 끊겨도 남지 않는다.
///
/// ★왜 함수 끝에서 지우는 한 줄로는 안 되나★: 이 파일의 단언은 **실패할 목적**으로 있고, 실패는
///   unwind 라 그 줄을 건너뛴다. 게이트가 한 번 빨개질 때마다 폴더가 하나씩 남는 구조였다. 임시 홈에는
///   실 홈에서 복사한 인증 파일이 들어가므로 그 누수는 자격증명 누수이기도 하다.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// ★이 저장소를 cwd 로 실 에이전트를 띄우지 않는다★ — 재는 것과 무관한 위험이 는다. 실 스폰의 cwd 는
    /// 전부 이 아래에서 만든다. ★항목마다 자기 폴더를 판다★ — 같은 경로를 나눠 쓰면 `-- --test-threads=4`
    /// 에서 한 항목의 `Drop` 이 **다른 항목이 지금 쓰고 있는 cwd** 를 지운다.
    fn new(tag: &str) -> Scratch {
        let path =
            std::env::temp_dir().join(format!("engram-backend-contract-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("임시 폴더 생성 실패");
        Scratch { path }
    }

    /// 테스트 홈은 시스템 temp 아래에 두지 않는다 — codex 0.153.4 는 그 자리를 홈으로 받으면 sandbox
    /// 헬퍼 생성을 거부하고 경고를 찍는다(실측). `CARGO_TARGET_TMPDIR` = cargo 가 통합 테스트에 주는
    /// `target/tmp` 라 gitignore 안이고 temp 밖이다.
    fn new_home(tag: &str) -> Scratch {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("backend-contract-home-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("임시 홈 생성 실패");
        Scratch { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Q8 이 재는 축(cwd 가 git repo 인가)을 다른 질문에 섞지 않으려고, Q8 을 뺀 나머지는 **git repo 인**
    /// 임시 폴더에서 돈다.
    fn git_init(self) -> Scratch {
        let init = run_capture(
            "git",
            &["init".to_string(), "-q".to_string()],
            &[],
            &self.path,
            PROCESS_TIMEOUT,
        );
        assert_eq!(
            init.exit_code,
            Some(0),
            "임시 git repo 초기화 실패: {init:?}"
        );
        self
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 이 백엔드의 상태 디렉터리를 **이 항목 전용 임시 홈**으로 돌리는 환경변수 한 쌍. 홈이 없는 백엔드면
/// 빈 목록이고, 그때는 실 사용자 설정이 그대로 쓰인다(그 사실이 [`LiveProbe::home_redirect`] 의 경고다).
///
/// 반환의 [`Scratch`] 를 **호출자가 들고 있어야 한다** — 떨어뜨리면 그 자리에서 홈이 지워진다.
fn sandbox_home(probe: &LiveProbe, tag: &str) -> (Option<Scratch>, Vec<(String, String)>) {
    let Some(redirect) = probe.home_redirect.as_ref() else {
        return (None, Vec::new());
    };

    let home = Scratch::new_home(tag);

    // 실 홈은 그 변수가 이미 걸려 있으면 그쪽, 아니면 사용자 홈 아래 기본 위치. ★읽기만 한다★.
    let real = std::env::var_os(redirect.env)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|h| PathBuf::from(h).join(redirect.default_subdir))
        });

    if let Some(real) = real.filter(|p| p.is_dir()) {
        for f in redirect.seed_files {
            let from = real.join(f);
            if from.is_file() {
                let _ = std::fs::copy(&from, home.path().join(f));
            }
        }
    }
    // 복사보다 나중이라 같은 이름이면 이쪽이 이긴다.
    for (name, body) in redirect.seed_writes {
        std::fs::write(home.path().join(name), body).expect("임시 홈 씨앗 파일 쓰기 실패");
    }

    let env = vec![(
        redirect.env.to_string(),
        home.path().to_string_lossy().into_owned(),
    )];
    (Some(home), env)
}

fn interactive_spec(
    probe: &LiveProbe,
    cwd: &Path,
    env: &[(String, String)],
    extra: &[&str],
) -> CommandSpec {
    let mut args: Vec<&str> = probe.interactive_args.to_vec();
    args.extend_from_slice(extra);
    let (program, args) = console_wrapped(probe.program, &args);
    CommandSpec {
        program,
        args,
        env: env.to_vec(),
        cwd: cwd.to_path_buf(),
    }
}

/// 시작 모달을 지나려 한 결과. ★`bool` 이 아닌 이유★: 「안 떴다」와 「이 백엔드엔 그런 모달이 없다」는
/// 같은 값이 아니다 — 앞쪽은 그 질문의 전제가 흔들린 것이고 뒤쪽은 애초에 물을 것이 없는 것이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModalOutcome {
    /// 표에 모달 표식이 없는 백엔드.
    NotDeclared,
    /// 표식은 있는데 상한 안에 화면에 안 나타났다.
    NotShown,
    /// 떠 있었고, 기본 선택으로 지나갔다.
    Passed,
}

/// 실 PTY 위의 한 화신. `Drop` 이 teardown 을 진다 — 단언이 패닉으로 끊겨도 자식이 남지 않는다.
struct PtySession {
    transport: Box<dyn AgentTransport>,
    core: Arc<OutputCore>,
    sink: RecordingSink,
    child_pid: Option<u32>,
    down: bool,
}

impl PtySession {
    fn open(spec: &CommandSpec, cols: u16, rows: u16) -> PtySession {
        let (pty, child_pid) =
            PtyTransport::open(spec, cols, rows).unwrap_or_else(|e| panic!("PTY open 실패: {e:?}"));
        let status_sink: Arc<dyn StatusSink> = Arc::new(NoopStatusSink);
        let core = Arc::new(OutputCore::new(
            Uuid::new_v4(),
            0,
            status_sink,
            TurnWiring::detached(),
        ));
        let transport: Box<dyn AgentTransport> = Box::new(pty);
        transport.start(core.clone());
        let sink = RecordingSink::new();
        let _sid = core.subscribe(Arc::new(sink.clone()));
        PtySession {
            transport,
            core,
            sink,
            child_pid,
            down: false,
        }
    }

    fn write(&self, bytes: &[u8]) {
        self.transport
            .send_input(InputEvent::Raw(bytes.to_vec()))
            .expect("send_input 실패");
    }

    /// ADR-0001 의 2동사 그대로: `shutdown()` → `join_pump(5s)`. 멱등이라 `Drop` 과 겹쳐도 된다.
    fn teardown(&mut self) {
        let _ = self.teardown_measured();
    }

    /// 같은 2동사를 **각각 재면서** 부른다. 반환 = `(shutdown 소요, join_pump 소요)`. 이미 내렸으면 `None`.
    ///
    /// ★왜 시간을 재나 — 이게 Q10 의 둘째 동사를 관측하는 유일한 창이다★: [`OutputCore::join_pump`] 는
    ///   `()` 를 돌려주고 안에서 `recv_timeout` 의 결과를 **버린다**(`src/output_core.rs:383-392`). 그래서
    ///   "pump 가 끝났다" 와 "5초를 그냥 기다렸다" 가 호출자에게 **똑같이 보인다**. 소요 시간만이 둘을
    ///   가른다 — 상한에 눌어붙었으면 pump 는 안 끝난 것이다.
    fn teardown_measured(&mut self) -> Option<(Duration, Duration)> {
        if self.down {
            return None;
        }
        self.down = true;
        let t0 = Instant::now();
        self.transport.shutdown();
        let shutdown_took = t0.elapsed();
        let t1 = Instant::now();
        self.core.join_pump(JOIN_PUMP_TIMEOUT);
        Some((shutdown_took, t1.elapsed()))
    }

    fn wait_first_bytes(&self) -> bool {
        wait_until(FIRST_BYTES_TIMEOUT, || self.sink.len() > 0)
    }

    /// 지금까지 온 바이트 전부를 사람이 읽는 글자로.
    fn screen(&self) -> String {
        visible_text(&self.sink.snapshot())
    }

    /// `from` 이후에 온 바이트만 사람이 읽는 글자로.
    fn tail_text(&self, from: usize) -> String {
        visible_text(&self.sink.tail_from(from))
    }

    /// `from` 이후 출력에 `needle` 이 나타날 때까지 기다린다. ★[`flatten`] 을 거친다★ — TUI 는 박스 안에서
    /// 줄을 접고 여백으로 정렬하므로 raw 텍스트에 그대로 대면 접힌 자리에서 어긋난다.
    fn wait_for_text(&self, from: usize, needle: &str, timeout: Duration) -> bool {
        wait_until(timeout, || flatten(&self.tail_text(from)).contains(needle))
    }

    /// 시작 모달이 떠 있으면 **기본 선택으로 한 번 지나간다**.
    ///
    /// ★추측이 아니다★: codex 0.153.4 의 폴더 신뢰 모달은 화면에 `1. Yes, continue` 를 선택해 두고
    ///   `Press enter to continue` 라고 직접 적는다(실측). 그래서 CR 하나가 그 모달의 문서화된 조작이다.
    /// ★대가★: 그 선택은 그 CLI 의 설정 파일에 폴더 신뢰 항목을 남긴다 — 그래서 이 동사를 쓰는 항목은
    ///   전부 [`sandbox_home`] 이 판 임시 홈으로 돌려 놓고 부른다. 실 사용자 설정은 안 바뀐다.
    ///
    /// 반환의 둘째 칸 = **CR 을 쓴 직후의 누적 길이**. ★그 위치가 필요한 이유★: 이 TUI 는 모달을 컴포저
    /// **위에 겹쳐** 그리므로 빈 컴포저 안내가 모달보다 **먼저** 화면에 있다(실측 — Q8 화면 덤프). 그 앞
    /// 구간을 포함해 안내 문구를 찾으면 "모달을 지나서 컴포저가 섰다" 가 아니라 "모달 뒤에 가려진 컴포저를
    /// 봤다" 가 되고, 그건 아무것도 안 묻는 판정이다.
    fn pass_startup_modal(&self, marker: Option<&str>) -> (ModalOutcome, usize) {
        let Some(marker) = marker else {
            return (ModalOutcome::NotDeclared, self.sink.len());
        };
        // ★한 번 보고 마는 대신 기다린다★: 모달은 첫 프레임에 안 나오는 경우가 있고, 그것을 "안 떴다" 로
        //   읽으면 그 뒤 질문 전체가 모달 뒤에서 물어보는 꼴이 된다.
        if !wait_until(MODAL_TIMEOUT, || flatten(&self.screen()).contains(marker)) {
            return (ModalOutcome::NotShown, self.sink.len());
        }
        self.write(b"\r");
        let after_cr = self.sink.len();
        self.wait_quiet();
        (ModalOutcome::Passed, after_cr)
    }

    /// 실 TUI 는 뜬 뒤에도 한동안 다시 그린다. 첫 바이트만 보고 입력을 넣으면 컴포저가 아직 없을 수
    /// 있어, **출력이 멎을 때까지** 기다린 뒤 다음 단계로 간다. 반환 = 멎은 시점의 누적 길이.
    fn wait_quiet(&self) -> usize {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        let mut last = self.sink.len();
        let mut since = Instant::now();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            let now = self.sink.len();
            if now != last {
                last = now;
                since = Instant::now();
            } else if since.elapsed() >= SETTLE_QUIET {
                break;
            }
        }
        last
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// 모달을 지난 뒤 **컴포저가 실제로 설 때까지** 기다리고, 안 서면 그 자리에서 멈춘다. 반환 = 그 신호를
/// 가진 백엔드면 `Some(true)`, 표에 신호가 없으면 `None`.
///
/// ★컴포저를 묻기 전에 컴포저가 있는지부터 묻는다★: 안 그러면 키 입력은 컴포저가 아닌 무엇(다음 모달·
///   로딩 화면)으로 가고, 그때 나오는 "에코가 없다" 는 **이 질문의 답이 아니라 하네스의 오작동**인데 둘이
///   똑같이 빨갛다. 실제로 codex 는 빈 홈에서 모달을 하나 더 띄웠고, 그때 Q5 는 「조각이 컴포저에 안
///   보인다」고 — 즉 답을 아는 것처럼 — 죽었다. 이 게이트가 그 둘을 갈라 준다.
fn wait_for_composer(s: &PtySession, probe: &LiveProbe, from: usize, name: &str) -> Option<bool> {
    let marker = probe.composer_idle_marker?;
    let ready = s.wait_for_text(from, marker, COMPOSER_TIMEOUT);
    assert!(
        ready,
        "{name}: 시작 모달을 지난 뒤 {COMPOSER_TIMEOUT:?} 안에 컴포저(`{marker}`)가 서지 않았다 — \
         지금 화면에 있는 것은 컴포저가 아니다. 여기서 재는 것은 컴포저 계약이므로 그 위에 다른 화면이 \
         떠 있으면 아래 판정은 전부 무의미하다: {}",
        excerpt(&s.screen(), 1200)
    );
    Some(ready)
}

/// TUI 출력은 대부분 제어 시퀀스다. "무엇이 새로 왔나" 를 사람이 읽는 글자로만 재려면 먼저 뗀다.
fn visible_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut chars = text.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if c == '\n' || c == '\t' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI — 최종 바이트 `@`..`~` 까지.
            Some('[') => {
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            // OSC 및 문자열 시퀀스 — BEL 또는 ST(ESC \) 까지.
            Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                let mut prev_esc = false;
                for f in chars.by_ref() {
                    if f == '\u{7}' || (prev_esc && f == '\\') {
                        break;
                    }
                    prev_esc = f == '\u{1b}';
                }
            }
            _ => {}
        }
    }
    out
}

/// `text` 에서 `needles` 를 걷어내고 남는 **읽을 수 있는 글자**. 비어 있지 않으면 "그 텍스트가 아닌
/// 새 출력" 이 왔다는 뜻이다(TRD §3-3 의 간접 판정 ②).
fn printable_residue(text: &str, needles: &[&str]) -> String {
    let mut cleaned = text.to_string();
    for n in needles {
        cleaned = cleaned.replace(n, " ");
    }
    cleaned.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 화면 텍스트를 줄바꿈·정렬 공백 없는 한 줄로. TUI 는 박스 안에서 줄을 접으므로, 표식을 raw 텍스트에
/// 그대로 대면 접힌 자리에서 어긋난다.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `parts` 가 **사이에 아무것도 끼지 않고 잇달아** 나타나는 자리. 반환 = 그 앞뒤를 조금 붙인 발췌.
///
/// ★Q5 가 재는 것이 정확히 이것이다★: 조각들이 화면 어딘가에 다 보이는 것과 **컴포저에 이어 붙어** 있는
///   것은 다른 사실이다. 앞쪽은 줄마다 제출돼 응답 사이사이에 흩어져 있어도 참이 되므로, 그 판정으로는
///   "여러 줄이 통째로 들어갔다" 를 주장할 수 없다 — 쪼개져 제출됐다면 조각 **사이에** 모델 응답·스피너·
///   프롬프트 표식이 그려져 인접이 깨진다. 실측된 결론(`› hi there`)이 이 술어의 모양이다.
///
/// ★한계를 정직하게 적는다 — 「같은 화면 줄인가」는 이 바이트 스트림으로 못 잰다★: TUI 는 줄바꿈 문자가
///   아니라 CSI 커서 이동으로 행을 옮기므로 [`visible_text`] 의 `\n` 은 **화면 행과 대응하지 않는다**(실측:
///   `› hi there` 로 한 줄에 그려진 것이 텍스트에서는 두 줄로 끊겨 있었다). 화면 행을 복원하려면 터미널
///   에뮬레이터가 있어야 한다. 그래서 이 술어는 「같은 행」이 아니라 **「사이에 다른 것이 없다」**를 잰다.
fn contiguous_echo(text: &str, parts: &[&str]) -> Option<String> {
    let flat = flatten(text);
    // 컴포저가 조각을 공백으로 잇든(줄바꿈이 공백으로 접힌 경우) 붙여 놓든 둘 다 "사이에 아무것도 없다".
    for sep in [" ", ""] {
        let joined = parts.join(sep);
        if joined.is_empty() {
            continue;
        }
        if let Some(at) = flat.find(&joined) {
            let from = flat[..at]
                .char_indices()
                .rev()
                .nth(24)
                .map_or(0, |(i, _)| i);
            return Some(flat[from..].chars().take(80).collect());
        }
    }
    None
}

fn excerpt(text: &str, max: usize) -> String {
    let flat = flatten(text);
    if flat.chars().count() <= max {
        flat
    } else {
        let head: String = flat.chars().take(max).collect();
        format!("{head}...")
    }
}

fn contains_seq(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// 출력에 든 DEC private mode set/reset(`ESC [ ? <params> <h|l>`) 전부, 중복 없이 나온 순서대로.
/// Q6 이 "두 모드의 바이트가 어디서 갈리나" 를 사람에게 보여 주는 증거다.
fn private_modes(raw: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 3 < raw.len() {
        if raw[i] != 0x1b || raw[i + 1] != b'[' || raw[i + 2] != b'?' {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while j < raw.len() && (raw[j].is_ascii_digit() || raw[j] == b';') {
            j += 1;
        }
        if j < raw.len() && (raw[j] == b'h' || raw[j] == b'l') {
            let seq = format!(
                "?{}{}",
                String::from_utf8_lossy(&raw[i + 3..j]),
                raw[j] as char
            );
            if !out.contains(&seq) {
                out.push(seq);
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// `root` 아래 살아 있는 후손 PID 전부(root 자신은 뺀다).
fn descendants(root: u32) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for c in engram_dashboard_base::platform::child_pids(p) {
            if !out.contains(&c) {
                out.push(c);
                frontier.push(c);
            }
        }
    }
    out
}

fn pid_alive(pid: u32) -> bool {
    engram_dashboard_base::platform::pid_alive(pid)
}

/// `known` 과 `root` 중 아직 살아 있는 것 전부 — ★살아 있는 것 아래를 다시 훑어★ 앞선 열거 **뒤에** 생긴
/// 자손까지 잡는다.
///
/// ★왜 스냅샷 재확인으로는 부족한가★: shutdown 직전에 한 번 세어 둔 명단만 다시 보면, 그 열거 뒤에 태어난
///   손자는 **아무도 안 본 채로 살아남는다**. Q10 이 재는 것이 "트리가 비었나" 인데 그 구멍으로는 안 빈
///   트리가 초록이 된다.
/// ★남는 한계는 적어 둔다★: 부모가 이미 죽어 버린 고아는 이 트리 걷기로 못 찾는다 — 이 술어는 사슬이
///   끊기지 않은 후손만 본다.
fn alive_survivors(root: u32, known: &[u32]) -> Vec<u32> {
    let mut seen: Vec<u32> = std::iter::once(root).chain(known.iter().copied()).collect();
    let mut i = 0usize;
    while i < seen.len() {
        let p = seen[i];
        i += 1;
        if !pid_alive(p) {
            continue;
        }
        for c in descendants(p) {
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    seen.retain(|p| pid_alive(*p));
    seen
}

// ── Q1 — 실 PTY 아래서 뜨는가 (★게이트★) ────────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q1_comes_up_under_real_pty() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q1").git_init();
        let (_home, env) = sandbox_home(probe, "q1");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        let started = Instant::now();
        let got = s.wait_first_bytes();
        let first_at = started.elapsed();
        let len = s.sink.len();
        let screen = s.screen();
        s.teardown();

        println!(
            "[Q1 {}] 첫 바이트 {}: {:?} 만에 {len} bytes\n  화면: {}",
            row.name,
            if got { "수신" } else { "미수신" },
            first_at,
            excerpt(&screen, 400)
        );
        assert!(
            got,
            "{}: {FIRST_BYTES_TIMEOUT:?} 안에 첫 바이트가 없다 — ★§3-5 게이트★ Phase 1 전체가 무너진다",
            row.name
        );
    }
}

// ── Q2 — 첫 출력 뒤 살아 있는가 ─────────────────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q2_stays_alive_after_first_output() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q2").git_init();
        let (_home, env) = sandbox_home(probe, "q2");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let len = s.sink.len();
        s.teardown();

        println!(
            "[Q2 {}] 첫 출력 뒤 {ALIVE_WINDOW:?} 시점 status={status:?} · 누적 {len} bytes",
            row.name
        );
        assert!(
            got_first,
            "{}: 첫 바이트가 없어 Q2 를 물을 수 없다",
            row.name
        );
        assert!(
            matches!(status, AgentStatus::Running),
            "{}: 첫 출력 뒤 {ALIVE_WINDOW:?} 안에 종료했다 — status={status:?}",
            row.name
        );
    }
}

// ── Q3 — 파이프(비-TTY) stdin ───────────────────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q3_non_tty_stdin_is_refused() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q3").git_init();
        let (_home, env) = sandbox_home(probe, "q3");
        let (program, args) = console_wrapped(probe.program, probe.interactive_args);
        let out = run_capture(&program, &args, &env, cwd.path(), PROCESS_TIMEOUT);
        let said = format!("{}{}", out.stdout, out.stderr);

        println!(
            "[Q3 {}] exit={:?} timed_out={} elapsed={:?}\n  stderr: {}\n  stdout: {}",
            row.name,
            out.exit_code,
            out.timed_out,
            out.elapsed,
            excerpt(&out.stderr, 300),
            excerpt(&out.stdout, 300)
        );
        assert!(
            !out.timed_out,
            "{}: 비-TTY stdin 인데 {PROCESS_TIMEOUT:?} 안에 끝나지 않았다 — 대화형이 그대로 떴다는 뜻",
            row.name
        );
        assert_ne!(
            out.exit_code,
            Some(0),
            "{}: 비-TTY stdin 을 성공으로 받았다 — PTY 가 유일한 대화형 경로라는 전제가 흔들린다",
            row.name
        );
        // ★종료코드만으로는 이 질문을 못 판정한다★: 우리는 세 fd 를 **전부** 파이프로 주므로, 0 이 아닌
        //   종료는 "stdin 이 tty 가 아니라서" 일 수도 있고 인증 실패·설정 오류·패닉일 수도 있다. 어느 쪽이든
        //   똑같이 초록이던 것이 옛 모양이다. 그 CLI 가 직접 하는 말이 둘을 가르는 유일한 증거다.
        //   ★남는 confound 는 적어 둔다★ — stdout/stderr 도 파이프라, "stdin 만" 을 격리하려면 그 둘을 PTY
        //   로 주는 하네스가 따로 필요하다. 이 단언은 거기까지 가지 않고 **문구**로 축을 고정한다.
        if let Some(marker) = probe.stdin_refusal_marker {
            assert!(
                said.contains(marker),
                "{}: 0 이 아닌 종료는 했는데 `{marker}` 를 말하지 않았다 — 이 죽음이 stdin 때문이라는 \
                 증거가 없다(실측 M1 이 바뀌었거나 다른 이유로 죽은 것): {out:?}",
                row.name
            );
        } else {
            println!(
                "[Q3 {}] ★거부 문구가 표에 없다 — 이 행은 종료코드만 본다(약한 판정)★",
                row.name
            );
        }
    }
}

// ── Q4 — 본문 + 간격 + CR 이 제출로 읽히는가 (★게이트★) ────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q4_body_pause_cr_reads_as_submit() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q4").git_init();
        let (_home, env) = sandbox_home(probe, "q4");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();
        let (modal, after_modal) = s.pass_startup_modal(probe.startup_modal_marker);
        let composer = wait_for_composer(&s, probe, after_modal, row.name);

        // TUI 는 입력이 없어도 다시 그린다. 그 바닥 소음을 먼저 재 둔다 — **단언 기준이 아니다**(아래 판정은
        // 「컴포저가 비었나」로 선다). 보고서가 "이 관측이 얼마나 흔들리는 값인가" 를 말하게 하는 숫자다.
        let idle_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let idle_noise = printable_residue(&s.tail_text(idle_mark), &[]);

        let body_mark = s.sink.len();
        s.write(probe.submit_body.as_bytes());
        let wrote_at = Instant::now();
        let echoed = wait_until(ECHO_TIMEOUT, || {
            flatten(&s.tail_text(body_mark)).contains(probe.submit_body)
        });
        let echo_text = s.tail_text(body_mark);
        // ★운영 계약 그대로 — 본문 write 와 제출 write 사이의 간격★. 위 에코 대기가 그 간격을 이미
        //   넘겼으면 더 자지 않고, 모자란 만큼만 채운다. 재는 것이 「본문 + **간격** + CR」이라 이 하한은
        //   못 줄인다(`SUBMIT_PACING` 주석의 실측이 그 근거).
        if let Some(rest) = SUBMIT_PACING.checked_sub(wrote_at.elapsed()) {
            std::thread::sleep(rest);
        }

        let cr_mark = s.sink.len();
        let submit = InputEncoder::Raw
            .submit_sequence()
            .expect("Raw encoder 의 제출 바이트가 없다");
        s.write(submit);

        // ★판정은 「컴포저가 비었나」다 — 「뭔가 새로 그려졌나」가 아니다★: TUI 는 아무 입력 없이도 배너·
        //   모델명·사용량을 다시 그리므로(바로 위 `idle_noise` 가 그 값을 찍는다) 후자는 제출의 증거가 못
        //   된다. 제출은 컴포저를 비우고, 빈 컴포저 자리에는 안내 문구가 **다시** 그려진다 — 그 문구의
        //   재등장이 이 질문이 실제로 관측한 사실이고, 기록된 결론("컴포저가 비었다")과 같은 것이다.
        let cleared = match probe.composer_idle_marker {
            Some(m) => s.wait_for_text(cr_mark, m, RESPONSE_TIMEOUT),
            // 그 신호가 없는 백엔드는 약한 판정으로 내려간다 — 본문 아닌 글자가 왔나. 위 소음 항목이 말하듯
            // 이 판정은 흔들리므로, 그 사실을 조용히 넘기지 않고 아래 출력에 적는다.
            None => wait_until(RESPONSE_TIMEOUT, || {
                !printable_residue(&s.tail_text(cr_mark), &[probe.submit_body]).is_empty()
            }),
        };
        let after_cr_bytes = s.sink.len().saturating_sub(cr_mark);
        let after_cr_text = s.tail_text(cr_mark);
        s.teardown();

        println!(
            "[Q4 {}] 모달={modal:?}·컴포저={composer:?} · 에코={echoed} · CR 뒤 컴포저 비움={cleared} \
             ({after_cr_bytes} bytes 새로 옴, 판정축={})\n  바닥 소음({PRE_SUBMIT_WINDOW:?} 무입력)={} \
             chars\n  에코 화면: {}\n  CR 뒤 화면: {}",
            row.name,
            match probe.composer_idle_marker {
                Some(m) => format!("빈 컴포저 안내 `{m}` 재등장"),
                None => "★약한 판정★ 본문 아닌 글자 등장".to_string(),
            },
            idle_noise.chars().count(),
            excerpt(&echo_text, 300),
            excerpt(&after_cr_text, 600)
        );
        assert!(
            echoed,
            "{}: 본문이 컴포저에 에코되지 않았다 — 판정 ① 실패라 ②③ 을 물을 수 없다",
            row.name
        );
        assert!(
            cleared,
            "{}: CR 뒤 {RESPONSE_TIMEOUT:?} 안에 컴포저가 비지 않았다 — ★§3-5 게이트★ \
             제출 기제가 백엔드별로 갈린다는 뜻이라 InputEncoder 변형이 하나 더 필요해진다",
            row.name
        );
    }
}

// ── Q5 — 여러 줄 본문이 통째로 들어가는가 (★게이트★) ───────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q5_multiline_body_enters_whole() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);
        let body = probe.multiline_parts.join("\n");

        let cwd = Scratch::new("q5").git_init();
        let (_home, env) = sandbox_home(probe, "q5");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();
        let (modal, after_modal) = s.pass_startup_modal(probe.startup_modal_marker);
        let composer = wait_for_composer(&s, probe, after_modal, row.name);

        // ★CR 을 보내지 않는다★: 본문에 든 LF 만으로 제출이 일어나는지가 이 질문이다.
        let mark = s.sink.len();
        s.write(body.as_bytes());
        // 조각이 **사이에 아무것도 없이 이어 붙었나** 를 기다린다 — 고정 sleep 이면 느린 첫 프레임 하나에
        // 오판한다.
        let joined = wait_until(ECHO_TIMEOUT, || {
            contiguous_echo(&s.tail_text(mark), probe.multiline_parts).is_some()
        });
        let echo_text = s.tail_text(mark);
        let composer_line = contiguous_echo(&echo_text, probe.multiline_parts);
        let missing: Vec<&str> = probe
            .multiline_parts
            .iter()
            .copied()
            .filter(|p| !flatten(&echo_text).contains(p))
            .collect();

        // 에코를 확인한 **뒤부터** 다시 센다. 앞 구간에는 본문을 넣기 전 TUI 가 그리던 프레임이 섞여
        // 있어, 그때의 빈 컴포저 안내가 "제출돼서 비었다" 로 오독된다.
        let after_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let after_bytes = s.sink.len().saturating_sub(after_mark);
        let after = s.tail_text(after_mark);
        // ★「정적」의 단위는 바이트가 아니라 **읽을 수 있는 글자**다★: 바이트로 재면 커서 깜빡임 같은 순수
        //   제어 시퀀스가 "화면이 움직였다" 로 잡힌다(실측 — 이 창에서 48 bytes 가 왔는데 그중 글자는 0자).
        //   제출의 증거가 되는 것은 새로 **그려진 글자**이지 커서가 깜빡였다는 사실이 아니다. 본문 자체는
        //   needle 로 빼 둔다 — 컴포저를 다시 그리면서 본문이 재등장하는 것은 제출의 반대 증거다.
        let after_visible = printable_residue(&after, probe.multiline_parts);
        // ★약속된 폴백을 실제로 구현한다★: 빈 컴포저 안내가 없는 백엔드는 「화면 정적 여부만 본다」고
        //   [`LiveProbe::composer_idle_marker`] 의 주석이 약속하는데, `is_some_and` 는 그 경우 **무조건
        //   false** 라 그 백엔드의 Q5 는 영구 초록이었다. 없는 신호를 없다고 적는 것과, 없을 때 다른 것을
        //   본다고 적어 놓고 아무것도 안 보는 것은 다르다.
        let submitted = match probe.composer_idle_marker {
            Some(m) => flatten(&after).contains(m),
            None => !after_visible.is_empty(),
        };
        s.teardown();

        println!(
            "[Q5 {}] 모달={modal:?}·컴포저={composer:?} · 미에코 조각={missing:?} · 인접={joined} · \
             CR 전 제출={submitted}\n  인접 자리: {}\n  에코 화면: {}\n  \
             이후 화면({PRE_SUBMIT_WINDOW:?}, 새 바이트 {after_bytes} 중 글자 {}자): {}",
            row.name,
            composer_line.as_deref().unwrap_or("(못 찾음)"),
            excerpt(&echo_text, 400),
            after_visible.chars().count(),
            excerpt(&after, 500)
        );
        assert!(
            missing.is_empty(),
            "{}: 여러 줄 본문의 조각 {missing:?} 가 컴포저에 안 보인다 — 통째로 들어가지 않았다",
            row.name
        );
        assert!(
            joined,
            "{}: 조각들이 화면에 다 있긴 한데 **사이에 다른 것이 끼어** 있다 — 흩어져 있으면 '통째로 \
             들어갔다' 를 주장할 수 없다(줄마다 제출돼 응답 사이에 흩어진 모양도 조각은 다 보인다). \
             기록된 결론은 조각이 이어 붙은 컴포저 한 줄(`› hi there`)이다",
            row.name
        );
        assert!(
            !submitted,
            "{}: CR 을 보내기 전에 컴포저가 비었다 — LF 가 제출로 읽힌다. ★§3-5 게이트★ \
             한 봉투가 여러 턴이 되면 Phase 2 우편이 그 위에 설 수 없고, 주입 경로에 \
             bracketed-paste 표식을 들이는 별건 설계가 선행돼야 한다",
            row.name
        );
        // ★기록된 결론은 「화면이 정적이었다」이고 그것도 지킨다★ — 바로 위 단언만 남기면 "빈 컴포저 안내가
        //   안 보였다" 까지만 지켜진다. 그 문구가 안 보여도 **다른** 글자가 새로 그려졌다면 무언가는 일어난
        //   것이고, 그때 기록된 근거는 더 이상 성립하지 않는다. 여기가 빨개지면 게이트가 뒤집힌 것이 아니라
        //   결론의 근거가 달라진 것이므로, 사람이 아래 화면을 읽고 §3-3 의 간접 판정을 다시 세워야 한다.
        assert!(
            after_visible.is_empty(),
            "{}: CR 없이 {PRE_SUBMIT_WINDOW:?} 동안 새 글자 {}자가 그려졌다({after_bytes} bytes). 빈 컴포저 \
             안내는 아직 안 보이지만 화면이 정적이지도 않다 — 기록된 근거가 더는 성립하지 않는다: {}",
            row.name,
            after_visible.chars().count(),
            excerpt(&after, 400)
        );
    }
}

// ── Q6 — `--no-alt-screen` 이 바이트 수준으로 다른가 ────────────────────────

/// alt-screen 진입 시퀀스 **계열** — xterm 의 1049 · 1047, 그리고 원조 47. 셋 중 하나라도 나오면
/// 그 모드는 화면을 통째로 바꿔치기하는 쪽이다.
const ALT_SCREEN_ENTERS: &[&[u8]] = &[b"\x1b[?1049h", b"\x1b[?1047h", b"\x1b[?47h"];

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q6_no_alt_screen_changes_the_bytes() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let Some(flag) = probe.no_alt_screen_arg else {
            println!(
                "[Q6 {}] alt-screen 을 끄는 인자가 표에 없다 — 이 행은 물을 것이 없다(건너뜀)",
                row.name
            );
            continue;
        };

        // ★두 포획은 **플래그 하나만** 달라야 한다★: 옛 모양은 두 실행이 같은 고정 경로를 써서 1회차는
        //   폴더 신뢰 모달을 그리고 2회차는 안 그렸다 — 차가운 PC 에서 그 차이가 통째로 `--no-alt-screen`
        //   의 효과로 기록된다. 지금은 포획마다 **새 cwd + 새 임시 홈**이라 양쪽 다 첫 방문이고, 모달
        //   결과가 어긋나면 아래에서 비교 자체를 멈춘다.
        //   (시작 모달이 떠 있는 동안은 모드 전환이 아직 안 일어났을 수 있으므로 둘 다 지나고 나서 잰다.)
        let mut seen: Vec<(bool, Vec<String>, usize, ModalOutcome)> = Vec::new();
        for extra in [Vec::new(), vec![flag]] {
            let cwd = Scratch::new("q6").git_init();
            let (_home, env) = sandbox_home(probe, "q6");
            let mut s =
                PtySession::open(&interactive_spec(probe, cwd.path(), &env, &extra), 80, 24);
            let got = s.wait_first_bytes();
            s.wait_quiet();
            let (modal, _) = s.pass_startup_modal(probe.startup_modal_marker);
            std::thread::sleep(CAPTURE_WINDOW);
            let raw = s.sink.snapshot();
            s.teardown();

            assert!(got, "{}: {extra:?} 조합에서 첫 바이트 없음", row.name);
            let alt = ALT_SCREEN_ENTERS.iter().any(|e| contains_seq(&raw, e));
            // ★집합으로 비교한다★ — `private_modes` 는 나온 **순서대로** 담으므로 `Vec` 을 그대로 대면
            //   프레임 순서가 바뀐 것만으로 "달라졌다" 가 된다. 이 질문이 묻는 것은 어느 모드를 쓰나이지
            //   어느 순서로 내나가 아니다.
            let mut modes = private_modes(&raw);
            modes.sort();
            modes.dedup();
            seen.push((alt, modes, raw.len(), modal));
        }

        println!(
            "[Q6 {}] 기본: alt_screen={} ({} bytes) 모달={:?} modes={:?}\n  {flag}: alt_screen={} \
             ({} bytes) 모달={:?} modes={:?}",
            row.name,
            seen[0].0,
            seen[0].2,
            seen[0].3,
            seen[0].1,
            seen[1].0,
            seen[1].2,
            seen[1].3,
            seen[1].1
        );
        assert_eq!(
            seen[0].3, seen[1].3,
            "{}: 두 포획의 시작 모달 결과가 다르다({:?} vs {:?}) — 플래그 말고 다른 것이 갈렸다는 뜻이라 \
             아래 비교는 `{flag}` 의 효과를 재는 것이 아니다",
            row.name, seen[0].3, seen[1].3
        );
        let changed = (seen[0].0, &seen[0].1) != (seen[1].0, &seen[1].1);
        assert_eq!(
            changed, probe.no_alt_screen_changes_bytes,
            "{}: `{flag}` 의 바이트 영향이 표에 적힌 값과 다르다 — 게이트는 아니지만 §4-11 의 권고가 \
             선 근거가 바뀌었다는 뜻이니 그 절을 다시 읽고 이 칸을 갱신한다",
            row.name
        );
    }
}

// ── Q8 — git repo 가 아닌 cwd (★게이트★) ───────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q8_non_git_cwd_is_accepted() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        // ★git init 을 하지 않는다★ — 그것이 이 질문의 축이다. 전제를 가정하지 않고 먼저 확인한다.
        let cwd = Scratch::new("q8-nongit");
        let inside = run_capture(
            "git",
            &["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            &[],
            cwd.path(),
            PROCESS_TIMEOUT,
        );
        assert_ne!(
            inside.exit_code,
            Some(0),
            "임시 폴더가 git work tree 안이다 — Q8 의 전제가 성립하지 않는다: {inside:?}"
        );

        let (_home, env) = sandbox_home(probe, "q8");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        s.wait_quiet();

        // ★여기가 이 항목의 전부다★: 처음 보는 폴더에는 신뢰 모달이 먼저 뜨고, 모달이 프로세스를 붙들고
        //   있는 동안 "5초 뒤에도 Running" 은 **거짓일 수가 없다**. 그 상태로 재면 「codex 가 git repo 아닌
        //   cwd 를 거부하지 않는다」는 반증 불가능한 주장이 되고, 이 게이트 행은 초록인 채로 아무것도 안
        //   묻는다. 모달을 지나고 **컴포저가 실제로 섰는지**까지 봐야 cwd 정책을 물은 것이다.
        let (modal, after_modal) = s.pass_startup_modal(probe.startup_modal_marker);
        // ★`after_modal` 부터 센다★ — 모달은 컴포저 **위에** 겹쳐 뜨므로 그 앞 구간에 이미 빈 컴포저
        //   안내가 그려져 있다(실측 화면 덤프). 거기서부터 찾으면 모달을 지나기 전에 그려진 것을 보고
        //   "섰다" 고 말하게 된다.
        let composer_ready = probe
            .composer_idle_marker
            .map(|m| s.wait_for_text(after_modal, m, COMPOSER_TIMEOUT));

        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let screen = s.screen();
        s.teardown();

        println!(
            "[Q8 {}] 첫 바이트={got_first} · 모달={modal:?} · 컴포저 기립={} · {ALIVE_WINDOW:?} 뒤 \
             status={status:?}\n  화면: {}",
            row.name,
            match composer_ready {
                Some(v) => v.to_string(),
                None => "표에 컴포저 표식 없음(★약한 판정★)".to_string(),
            },
            excerpt(&screen, 1200)
        );
        assert!(
            got_first,
            "{}: git repo 아닌 cwd 에서 아무 출력도 없다 — ★§3-5 게이트★",
            row.name
        );
        if probe.startup_modal_marker.is_some() {
            assert_ne!(
                modal,
                ModalOutcome::NotShown,
                "{}: git repo 아닌 cwd 에서 {MODAL_TIMEOUT:?} 안에 아는 시작 모달이 안 떴다 — 이 폴더에서는 \
                 **다른 화면**(거부·경고·다른 모달)이 떴다는 뜻이라 아래 판정을 그대로 쓸 수 없다. \
                 사람이 화면을 읽고 표식을 갱신하거나 Q8 의 결론을 다시 써야 한다: {}",
                row.name,
                excerpt(&screen, 800)
            );
        }
        if let Some(ready) = composer_ready {
            assert!(
                ready,
                "{}: 모달을 지났는데 {COMPOSER_TIMEOUT:?} 안에 컴포저가 서지 않았다 — ★§3-5 게이트★ \
                 git repo 아닌 cwd 를 받아들이지 않았다는 뜻이다. 우리 스폰 입구는 임의 폴더를 받으므로 \
                 cwd 정책이 배선보다 먼저 결정돼야 한다: {}",
                row.name,
                excerpt(&screen, 800)
            );
        }
        assert!(
            matches!(status, AgentStatus::Running),
            "{}: git repo 아닌 cwd 를 거부했다(status={status:?}) — ★§3-5 게이트★ 우리 스폰 입구는 \
             임의 폴더를 받으므로 cwd 정책이 배선보다 먼저 결정돼야 한다",
            row.name
        );
    }
}

// ── Q9 — 호출자가 세션 id 를 정할 수 있는가 ─────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q9_caller_cannot_choose_the_session_id() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q9");
        let (_home, env) = sandbox_home(probe, "q9");
        let (program, args) = console_wrapped(probe.program, &["--help"]);
        let help_run = run_capture(&program, &args, &env, cwd.path(), PROCESS_TIMEOUT);

        assert_eq!(
            help_run.exit_code,
            Some(0),
            "{}: `--help` 가 실패했다: {help_run:?}",
            row.name
        );
        let help = format!("{}{}", help_run.stdout, help_run.stderr);

        match probe.session_id_flag {
            Some(flag) => {
                println!("[Q9 {}] 세션 id 플래그 `{flag}` 를 기대", row.name);
                assert!(
                    help.contains(flag),
                    "{}: `--help` 에 `{flag}` 가 없다 — 표가 있다고 적은 플래그가 사라졌다",
                    row.name
                );
            }
            None => {
                let found: Vec<&str> = SESSION_ID_CANDIDATES
                    .iter()
                    .copied()
                    .filter(|c| help.contains(c))
                    .collect();
                println!(
                    "[Q9 {}] `--help` {} bytes · 세션 id 후보 발견={found:?}",
                    row.name,
                    help.len()
                );
                assert!(
                    found.is_empty(),
                    "{}: 세션 id 를 정하는 플래그 {found:?} 가 실제로 있다 — 호출자가 sid 를 못 정한다는 \
                     판정이 뒤집힌다",
                    row.name
                );
            }
        }
    }
}

// ── Q10 — shutdown 뒤 프로세스 트리가 비는가 (★게이트★) ────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q10_shutdown_empties_the_process_tree() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = Scratch::new("q10").git_init();
        let (_home, env) = sandbox_home(probe, "q10");
        let mut s = PtySession::open(&interactive_spec(probe, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();

        let root = s.child_pid.expect("PTY 직속 자식 PID 를 못 받았다");
        // Job Object 에 배정되는 것은 직속 자식(`cmd.exe`)이고 실 CLI 는 그 손자·증손자다. 그 깊이가
        // 실제로 있는지 먼저 확인해야 "트리가 비었다" 가 공허한 단언이 되지 않는다.
        // ★본 명단을 **합집합으로 쌓는다**★ — 한 번 찍은 스냅샷만 들고 있으면 그 사이에 태어났다 죽은
        //   중간 고리가 명단에서 빠지고, 그 고리의 자식이 남아도 아무도 안 본다.
        let mut tree_before: Vec<u32> = Vec::new();
        let deep = wait_until(Duration::from_secs(15), || {
            for p in descendants(root) {
                if !tree_before.contains(&p) {
                    tree_before.push(p);
                }
            }
            !tree_before.is_empty()
        });
        let status_before = s.core.status();

        let (shutdown_took, join_took) = s
            .teardown_measured()
            .expect("이 항목이 teardown 을 처음 부른다");

        // ★열거를 다시 한다 — 옛 스냅샷만 재확인하지 않는다★: 위 열거 **뒤에** 태어난 손자는 스냅샷에
        //   없으므로, 명단만 다시 보면 살아남은 채로 초록이 된다.
        let mut alive_after: Vec<u32> = Vec::new();
        let gone = wait_until(Duration::from_secs(10), || {
            alive_after = alive_survivors(root, &tree_before);
            alive_after.is_empty()
        });
        let status_after = s.core.status();

        println!(
            "[Q10 {}] root={root} 후손={tree_before:?} · shutdown {shutdown_took:?} → \
             join_pump {join_took:?} · 생존={alive_after:?} · status {status_before:?} → {status_after:?}",
            row.name
        );
        assert!(
            deep || !cfg!(windows),
            "{}: 직속 자식 아래 후손이 하나도 안 보인다 — shim 사슬 전제가 흔들린다",
            row.name
        );
        assert!(
            gone,
            "{}: shutdown 뒤에도 {alive_after:?} 가 살아 있다 — ★§3-5 게이트★ 이건 회귀가 아니라 \
             kill 인과(ADR-0001) **첫 동사**의 위반이라 Job Object 배치를 다시 봐야 한다",
            row.name
        );

        // ── 여기부터가 둘째 동사다 ──────────────────────────────────────────────────
        //
        // ★PID 가 사라졌다는 것은 `shutdown()` 만 잰 것이다★. ADR-0001 의 인과는 거기서 안 끝난다 —
        //   master drop → reader EOF → pump break → `core.finish` → done_tx 가 그 뒤에 이어지고, terminal
        //   전이를 발행하는 것은 **pump 단독**이다(ADR-0005 · CLAUDE.md 「핵심 불변식」의 finalize 1회 ·
        //   상태 알림 분담). 그래서 pump 가 실제로 끝났다는 관측 가능한 결과는 **status 가 terminal 로
        //   넘어갔나**다: 안 넘어갔으면 `finish` 가 안 불렸고 = 루프가 안 깨졌다.
        // ★`finalized` 를 직접 볼 수는 없다★(공개 접근자가 없다). 하지만 `finish` 는 그 swap 을 통과한
        //   승자 경로에서만 status 를 쓰므로, terminal status 는 그 플래그가 섰다는 것과 같은 사실이다.
        assert!(
            !status_after.is_live(),
            "{}: 프로세스는 사라졌는데 status 가 아직 {status_after:?} 다 — pump 가 루프를 안 깼거나 \
             `finish` 가 안 불렸다는 뜻이다(terminal 전이는 pump 단독 — ADR-0005). ★§3-5 게이트★ \
             kill 인과(ADR-0001)의 **둘째 동사**가 성립하지 않는다",
            row.name
        );
        // ★상한에 눌어붙었나★ — `join_pump` 는 `recv_timeout` 의 결과를 버리므로(`output_core.rs:383-392`)
        //   "기다렸다 실패" 와 "바로 끝났다" 가 호출자에게 같은 `()` 로 보인다. 소요 시간만이 그 둘을 가른다.
        //   여유를 크게 두는 이유 = 이 단언이 재는 것은 "빠른가" 가 아니라 "상한까지 갔나" 다.
        assert!(
            join_took < JOIN_PUMP_TIMEOUT.mul_f32(0.9),
            "{}: join_pump 가 {join_took:?} 걸렸다 — 상한({JOIN_PUMP_TIMEOUT:?})에 눌어붙었다는 뜻이라 \
             pump 는 그 안에 안 끝났다. status 가 terminal 로 보이더라도 그건 상한 뒤에 늦게 온 것이다",
            row.name
        );
    }
}
