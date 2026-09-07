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
//!   확인이 컴포저 위에 겹쳐 뜨고, 그 동안 키 입력은 컴포저가 아니라 모달로 간다. 그래서 컴포저를 묻는
//!   질문(Q4·Q5·Q6)은 [`PtySession::pass_startup_modal`] 로 그것을 먼저 지나간다. **우리 스폰 입구는
//!   임의 폴더를 받으므로 이건 시험대만의 사정이 아니다** — 처음 보는 cwd 로 뜬 codex 탭은 사람이
//!   모달을 지나기 전까지 입력을 안 받는다.
//!
//! 실 자식 프로세스를 띄우므로 `-- --test-threads=1`(또는 4) 로 돈다 — 근거는 CLAUDE.md 「빌드·검증 명령」.

use std::io::Read;
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
    /// Q6 — alt-screen 을 끄는 인자.
    no_alt_screen_arg: &'static str,
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
            // `AgentCommand` 에 Codex variant 가 아직 없다(Phase 1 이 만든다). `CodexBackend` 의 선언
            // 메서드는 command 를 보지 않으므로 이 표본은 자리채움이고, variant 가 생기면 **이 한 줄만**
            // 바뀐다.
            sample: AgentCommand::Claude {
                extra_args: vec![],
                output_format: ClaudeOutputFormat::Terminal,
            },
            declared: Declared {
                // ★현재 stub 이 신고하는 값을 그대로 적는다 — 실측이 맞다고 도장 찍은 값이 아니다★:
                //   TRD §4-2 는 `needs_session` 을 false 로 뒤집으라 한다(호출자가 sid 를 못 정한다).
                //   Phase 1 이 codex.rs 를 고치면 이 항목이 빨개져 표 갱신을 강제한다 — 그것이 이 열의 일.
                needs_session: true,
                supports_control_channel: false,
                accepts_mcp_config: false,
                reads_messages: true,
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
                no_alt_screen_arg: "--no-alt-screen",
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

/// `backend::console_command` 와 **같은 래핑**. 그 함수는 `pub(crate)` 라 통합 테스트에서 못 부르므로
/// 여기서 다시 만든다 — 두 쪽이 갈리면 이 시험대는 운영이 실제로 띄우는 것과 다른 것을 재게 된다.
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
fn run_capture(program: &str, args: &[String], cwd: &Path, timeout: Duration) -> Captured {
    let started = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{program} spawn 실패: {e}"));

    drop(child.stdin.take());

    // ★파이프를 스레드로 빨아낸다★: `--help` 처럼 4KB 를 넘기는 출력은 파이프 버퍼가 차면 자식이
    //   write 에서 멈춰 종료 폴링이 영영 안 끝난다.
    let out_reader = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });
    let err_reader = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });

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
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = err_reader
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    Captured {
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        elapsed: started.elapsed(),
    }
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

    let found = run_capture(finder, &args, &std::env::temp_dir(), PROCESS_TIMEOUT);
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

/// ★이 저장소를 cwd 로 실 에이전트를 띄우지 않는다★ — 재는 것과 무관한 위험이 는다. 실 스폰은 전부
/// 이 아래 임시 폴더에서 돈다.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("engram-backend-contract-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("임시 폴더 생성 실패");
    dir
}

/// Q8 이 재는 축(cwd 가 git repo 인가)을 다른 질문에 섞지 않으려고, Q8 을 뺀 나머지는 **git repo 인**
/// 임시 폴더에서 돈다.
fn scratch_git_repo(tag: &str) -> PathBuf {
    git_init(scratch_dir(tag))
}

/// 컴포저를 묻는 질문(Q4·Q5·Q6) 전용 — **경로가 고정**이다.
///
/// ★왜 uuid 가 아닌가★: 이 질문들은 시작 모달을 지나가야 하고, 그 선택은 `~/.codex/config.toml` 에
///   폴더 신뢰 항목을 남긴다(실측). uuid 폴더로 돌면 실행할 때마다 **지워진 경로의 죽은 항목**이 하나씩
///   쌓인다. 고정 경로면 항목이 하나로 유지되고, 두 번째 실행부터는 모달 자체가 안 뜬다.
/// 내용물은 매번 새로 만든다 — 앞 실행이 남긴 것이 다음 실행에 섞이지 않게.
fn composer_scratch() -> PathBuf {
    let dir = std::env::temp_dir().join("engram-backend-contract-composer");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("임시 폴더 생성 실패");
    git_init(dir)
}

fn git_init(dir: PathBuf) -> PathBuf {
    let init = run_capture(
        "git",
        &["init".to_string(), "-q".to_string()],
        &dir,
        PROCESS_TIMEOUT,
    );
    assert_eq!(
        init.exit_code,
        Some(0),
        "임시 git repo 초기화 실패: {init:?}"
    );
    dir
}

fn remove_scratch(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn interactive_spec(probe: &LiveProbe, cwd: &Path, extra: &[&str]) -> CommandSpec {
    let mut args: Vec<&str> = probe.interactive_args.to_vec();
    args.extend_from_slice(extra);
    let (program, args) = console_wrapped(probe.program, &args);
    CommandSpec {
        program,
        args,
        env: vec![],
        cwd: cwd.to_path_buf(),
    }
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
        if self.down {
            return;
        }
        self.down = true;
        self.transport.shutdown();
        self.core.join_pump(Duration::from_secs(5));
    }

    fn wait_first_bytes(&self) -> bool {
        wait_until(FIRST_BYTES_TIMEOUT, || self.sink.len() > 0)
    }

    /// 시작 모달이 떠 있으면 **기본 선택으로 한 번 지나간다**. 반환 = 실제로 지나갔나.
    ///
    /// ★추측이 아니다★: codex 0.153.4 의 폴더 신뢰 모달은 화면에 `1. Yes, continue` 를 선택해 두고
    ///   `Press enter to continue` 라고 직접 적는다(실측). 그래서 CR 하나가 그 모달의 문서화된 조작이다.
    /// ★대가★: 그 선택은 `~/.codex/config.toml` 에 폴더 신뢰 항목을 남긴다 — 그래서 이 동사를 쓰는
    ///   질문들은 [`composer_scratch`] 의 고정 경로에서 돈다.
    fn pass_startup_modal(&self, marker: Option<&str>) -> bool {
        let Some(marker) = marker else {
            return false;
        };
        if !flatten(&visible_text(&self.sink.snapshot())).contains(marker) {
            return false;
        }
        self.write(b"\r");
        self.wait_quiet();
        true
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

// ── Q1 — 실 PTY 아래서 뜨는가 (★게이트★) ────────────────────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q1_comes_up_under_real_pty() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = scratch_git_repo("q1");
        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        let started = Instant::now();
        let got = s.wait_first_bytes();
        let first_at = started.elapsed();
        let len = s.sink.len();
        let screen = visible_text(&s.sink.snapshot());
        s.teardown();
        remove_scratch(&cwd);

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

        let cwd = scratch_git_repo("q2");
        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let len = s.sink.len();
        s.teardown();
        remove_scratch(&cwd);

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

        let cwd = scratch_git_repo("q3");
        let (program, args) = console_wrapped(probe.program, probe.interactive_args);
        let out = run_capture(&program, &args, &cwd, PROCESS_TIMEOUT);
        remove_scratch(&cwd);

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
    }
}

// ── Q4 — 본문 + 간격 + CR 이 제출로 읽히는가 (★게이트★) ────────────────────

#[test]
#[ignore = "실 codex 필요 — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q4_body_pause_cr_reads_as_submit() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(probe.program);

        let cwd = composer_scratch();
        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();
        let passed_modal = s.pass_startup_modal(probe.startup_modal_marker);

        // TUI 는 입력이 없어도 다시 그린다. 그 바닥 소음을 먼저 재 두면 판정 ② 의 긍정이 얼마나
        // 흔들리는 값인지 보고서가 말할 수 있다(단언 기준은 아니다 — TRD §3-3).
        let idle_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let idle_noise = printable_residue(&visible_text(&s.sink.tail_from(idle_mark)), &[]);

        let body_mark = s.sink.len();
        s.write(probe.submit_body.as_bytes());
        // 운영 계약 그대로 — 본문 write 와 제출 write 사이의 간격.
        std::thread::sleep(SUBMIT_PACING);
        let echo_text = visible_text(&s.sink.tail_from(body_mark));
        let echoed = echo_text.contains(probe.submit_body);

        let cr_mark = s.sink.len();
        let submit = InputEncoder::Raw
            .submit_sequence()
            .expect("Raw encoder 의 제출 바이트가 없다");
        s.write(submit);

        let sink = s.sink.clone();
        let body = probe.submit_body;
        let responded = wait_until(RESPONSE_TIMEOUT, || {
            !printable_residue(&visible_text(&sink.tail_from(cr_mark)), &[body]).is_empty()
        });
        let after_cr_bytes = s.sink.len().saturating_sub(cr_mark);
        let after_cr_text = visible_text(&s.sink.tail_from(cr_mark));
        s.teardown();
        remove_scratch(&cwd);

        println!(
            "[Q4 {}] 모달통과={passed_modal} · 에코={echoed} · CR 뒤 새 출력={responded} \
             ({after_cr_bytes} bytes)\n  바닥 소음({PRE_SUBMIT_WINDOW:?} 무입력)={} chars\n  \
             에코 화면: {}\n  CR 뒤 화면: {}",
            row.name,
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
            responded,
            "{}: CR 뒤 {RESPONSE_TIMEOUT:?} 동안 화면이 정적이다(판정 ③) — ★§3-5 게이트★ \
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

        let cwd = composer_scratch();
        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();
        let passed_modal = s.pass_startup_modal(probe.startup_modal_marker);

        // ★CR 을 보내지 않는다★: 본문에 든 LF 만으로 제출이 일어나는지가 이 질문이다.
        let mark = s.sink.len();
        s.write(body.as_bytes());
        std::thread::sleep(SUBMIT_PACING);

        let echo_text = visible_text(&s.sink.tail_from(mark));
        let missing: Vec<&str> = probe
            .multiline_parts
            .iter()
            .copied()
            .filter(|p| !echo_text.contains(p))
            .collect();

        // 에코를 확인한 **뒤부터** 다시 센다. 앞 구간에는 본문을 넣기 전 TUI 가 그리던 프레임이 섞여
        // 있어, 그때의 빈 컴포저 안내가 "제출돼서 비었다" 로 오독된다.
        let after_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let after = visible_text(&s.sink.tail_from(after_mark));
        let submitted = probe
            .composer_idle_marker
            .is_some_and(|m| flatten(&after).contains(m));
        let residue = printable_residue(&after, probe.multiline_parts);
        s.teardown();
        remove_scratch(&cwd);

        println!(
            "[Q5 {}] 모달통과={passed_modal} · 미에코 조각={missing:?} · CR 전 제출={submitted}\n  \
             에코 화면: {}\n  이후 화면({PRE_SUBMIT_WINDOW:?}, 잔여 {} chars): {}",
            row.name,
            excerpt(&echo_text, 400),
            residue.chars().count(),
            excerpt(&after, 500)
        );
        assert!(
            missing.is_empty(),
            "{}: 여러 줄 본문의 조각 {missing:?} 가 컴포저에 안 보인다 — 통째로 들어가지 않았다",
            row.name
        );
        assert!(
            !submitted,
            "{}: CR 을 보내기 전에 컴포저가 비었다 — LF 가 제출로 읽힌다. ★§3-5 게이트★ \
             한 봉투가 여러 턴이 되면 Phase 2 우편이 그 위에 설 수 없고, 주입 경로에 \
             bracketed-paste 표식을 들이는 별건 설계가 선행돼야 한다",
            row.name
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

        // 시작 모달이 떠 있는 동안은 모드 전환이 아직 안 일어났을 수 있다 — 두 모드 다 그것을 지나고
        // 나서 잰다.
        let mut seen: Vec<(bool, Vec<String>, usize)> = Vec::new();
        for extra in [Vec::new(), vec![probe.no_alt_screen_arg]] {
            let cwd = composer_scratch();
            let mut s = PtySession::open(&interactive_spec(probe, &cwd, &extra), 80, 24);
            let got = s.wait_first_bytes();
            s.wait_quiet();
            s.pass_startup_modal(probe.startup_modal_marker);
            std::thread::sleep(CAPTURE_WINDOW);
            let raw = s.sink.snapshot();
            s.teardown();
            remove_scratch(&cwd);

            assert!(got, "{}: {extra:?} 조합에서 첫 바이트 없음", row.name);
            let alt = ALT_SCREEN_ENTERS.iter().any(|e| contains_seq(&raw, e));
            seen.push((alt, private_modes(&raw), raw.len()));
        }

        println!(
            "[Q6 {}] 기본: alt_screen={} ({} bytes) private_modes={:?}\n  {}: alt_screen={} \
             ({} bytes) private_modes={:?}",
            row.name,
            seen[0].0,
            seen[0].2,
            seen[0].1,
            probe.no_alt_screen_arg,
            seen[1].0,
            seen[1].2,
            seen[1].1
        );
        let changed = (seen[0].0, &seen[0].1) != (seen[1].0, &seen[1].1);
        assert_eq!(
            changed, probe.no_alt_screen_changes_bytes,
            "{}: `{}` 의 바이트 영향이 표에 적힌 값과 다르다 — 게이트는 아니지만 §4-11 의 권고가 \
             선 근거가 바뀌었다는 뜻이니 그 절을 다시 읽고 이 칸을 갱신한다",
            row.name, probe.no_alt_screen_arg
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
        let cwd = scratch_dir("q8-nongit");
        let inside = run_capture(
            "git",
            &["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            &cwd,
            PROCESS_TIMEOUT,
        );
        assert_ne!(
            inside.exit_code,
            Some(0),
            "임시 폴더가 git work tree 안이다 — Q8 의 전제가 성립하지 않는다: {inside:?}"
        );

        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let screen = visible_text(&s.sink.snapshot());
        s.teardown();
        remove_scratch(&cwd);

        println!(
            "[Q8 {}] 첫 바이트={got_first} · {ALIVE_WINDOW:?} 뒤 status={status:?}\n  화면: {}",
            row.name,
            excerpt(&screen, 1200)
        );
        assert!(
            got_first,
            "{}: git repo 아닌 cwd 에서 아무 출력도 없다 — ★§3-5 게이트★",
            row.name
        );
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

        let cwd = scratch_dir("q9");
        let (program, args) = console_wrapped(probe.program, &["--help"]);
        let help_run = run_capture(&program, &args, &cwd, PROCESS_TIMEOUT);
        remove_scratch(&cwd);

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

        let cwd = scratch_git_repo("q10");
        let mut s = PtySession::open(&interactive_spec(probe, &cwd, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: 첫 바이트 없음", row.name);
        s.wait_quiet();

        let root = s.child_pid.expect("PTY 직속 자식 PID 를 못 받았다");
        // Job Object 에 배정되는 것은 직속 자식(`cmd.exe`)이고 실 CLI 는 그 손자·증손자다. 그 깊이가
        // 실제로 있는지 먼저 확인해야 "트리가 비었다" 가 공허한 단언이 되지 않는다.
        let mut tree = Vec::new();
        let deep = wait_until(Duration::from_secs(15), || {
            tree = descendants(root);
            !tree.is_empty()
        });
        let tree_before = tree.clone();

        s.teardown();

        let mut alive_after: Vec<u32> = Vec::new();
        let gone = wait_until(Duration::from_secs(10), || {
            alive_after = std::iter::once(root)
                .chain(tree_before.iter().copied())
                .filter(|p| pid_alive(*p))
                .collect();
            alive_after.is_empty()
        });
        remove_scratch(&cwd);

        println!(
            "[Q10 {}] root={root} 후손={tree_before:?} · shutdown 뒤 생존={alive_after:?}",
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
             kill 인과(ADR-0001) 불변식 위반이라 Job Object 배치를 다시 봐야 한다",
            row.name
        );
    }
}
