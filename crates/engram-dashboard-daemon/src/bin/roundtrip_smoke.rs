//! roundtrip-smoke — ADR-0092 A→B→A 왕복(reply round-trip) 실측 드라이버(검증 전용 bin).
//!
//! ## 역할
//! priming-smoke 는 A→B **수신**만 증명했다(합성 발신자 1명 → 실 에이전트 1명). 이 하네스는 그 위에
//! 두 가지를 **추가로** 실측한다:
//!   ① 실 에이전트 B 가 **발신 절반**(MCP `send_message` 툴 OR `engram-send` CLI)을 **스스로** 호출하고,
//!   ② A 가 B 의 답신을 자연스럽게 수용한다.
//!
//! 즉 실 primed claude **2개**(A·B, stream-json/Fresh)를 스폰하고:
//!   1. B 에게 짧은 원과제 턴을 줘 "일하는 팀원" 맥락을 만든다.
//!   2. A→B 로 자연스러운 질문 하나를 실 control 경로(`handle_send`, Entrance::Cli)로 **씨앗 주입**한다
//!      — B 는 봉투 `[message from alice id:..]` 에서 A 의 이름을 배운다(본문엔 "툴 X 를 써라" 같은 기계적
//!      지시를 넣지 않는다 — 발신 학습은 프라이밍 변형이 하고, 기본(no-priming/both) 은 순수 툴 발견을 본다).
//!   3. 하네스는 **B 의 답신에 대해 handle_send 를 부르지 않는다** — B(실 claude)가 스스로 MCP/CLI 입구를
//!      호출하고, 그 요청이 **실제 입구 → handle_send → wrap → A stdin** 으로 흐른다.
//!   4. 관측: (a) 기계적 = registry `DeliveryObservation`(from=B, to=A)이 실제로 생겼는지 + B 가 고른
//!      입구(Mcp/Cli). (b) 정성적 = A 의 `TurnObserver` 가 A 가 답신을 처리하며 낸 텍스트를 누적.
//!   5. 구조화 stdout 마커로 오케스트레이터 판정용 결과를 출력한다.
//!
//! ## 프라이밍 선택(발신 학습 변형 — priming 파일로 조절, ADR-0099 이후)
//!   - 미지정(또는 `--priming C0`) = `prompts/agent-priming.md`(운영 A = both-teaching: send_message +
//!     engram-send). "C0" 는 no-priming 기본과 동의어인 편의 별칭일 뿐이다(그 하나만 남았다).
//!   - `--priming <abs-or-rel-path>` 로 임의 프라이밍 파일 직접 지정(예: 운영 B `prompts/agent-priming-cli.md`).
//!     절대면 그대로, 상대면 repo 루트 기준.
//!   ※ 옛 C1~C3 케이스 별칭(`prompts/experiments/agent-priming-send-*.md` 로 매핑되던)은 ADR-0099 로 제거됐다
//!     — 실험 변형 파일이 정리됐고(git 이력 보존), 채널 변형은 이제 백엔드 capability 가 정적 2파일로 가른다.
//!     C1~C3 문자열을 `--priming` 에 넘기면 이제 "그 이름의 파일 경로"로 해석돼 하류에서 부재로 걸린다(특수
//!     매핑 없음).
//!
//! ## CLI-only 측정 노브 2종(측정 축이 다르다)
//!   - **`--disallow-mcp`(ADR-0094)** — `ENGRAM_DISALLOW_MCP_SEND` env 를 세워 MCP `send_message` **grant 만**
//!     뺀다. MCP 서버는 여전히 mcp-config 로 부착되고 프라이밍도 both-teaching 이라, 물리(MCP 살아있음)와
//!     교육/권한(CLI 만)이 **갈린다** — 프롬프트-도구 불일치를 일부러 만들어 순수 CLI 라우팅을 측정한다.
//!   - **`--cli-only`(ADR-0099 FIX 3)** — `ENGRAM_FORCE_CLI_ONLY_SEND` env 를 세워 provision 을 **비-MCP 로
//!     강제**한다. mcp-config 미부착 + CliOnly 프라이밍(`prompts/agent-priming-cli.md`) + [Cli] grant 가 함께
//!     움직여 정합 불변식을 **보존한 채** false path 전체를 실측한다(실 claude 를 비-MCP 백엔드처럼 굴림).
//!     이 모드는 `--priming` override 를 주지 않는다(상속된 `ENGRAM_PRIMING_FILE` env 도 거부 = SETUP-FAIL,
//!     조용히 clear 하지 않는다) — provision 이 CliOnly 파일을 auto-select 하는 걸 보는 게 목적이고,
//!     entrance=cli 를 기대한다(mcp 관측 시 SETUP-FAIL = 강제 seam 결함).
//!     ★엄격 판정(다른 모드와 다르다)★: `--cli-only` 는 B 가 실제로 CLI 입구로 보냈을 때만(b_sent=true AND
//!     entrance=cli) exit 0(PASS). 아무것도 안 보낸 경우(B_SENT=false/ENTRANCE=none)는 일반 모드의
//!     valid-negative 와 달리 **exit 1**(FAIL — 강제 false path 가 도는 걸 못 봤으니 목적 미달)이고, 전용
//!     `VERDICT [... --cli-only]:` 줄로 결과를 낸다.
//!   둘 다 test-only 노브(운영 스위치 아님)이고 CLI 입구(send_exe = engram-send 형제 빌드)가 필수다 — 없으면
//!   SETUP-SKIP. 미지정이면 오늘 동작(MCP 경로)과 바이트 동일.
//!
//! ## 실행(오케스트레이터가 런타임에 돌린다 — 이 파일은 빌드/컴파일만)
//! ★CLI 입구를 쓰는 실험(운영 B `prompts/agent-priming-cli.md` 또는 `--cli-only`)은 먼저 `engram-send` 를
//!   빌드해야 한다★ — 이 하네스는 자기 exe 형제에서 `engram-send`(Win: `.exe`) 를 찾아 CLI 입구를 켠다.
//!   형제에 없으면 B 가 그 경로로 못 보내 **인프라 부재를 실험적 negative 로 오인**할 수 있다. `cargo run` 은
//!   dep bin 을 안 만들므로 별도로 빌드한다(같은 profile/target 이어야 형제로 co-locate 된다):
//! ```text
//! # 1) CLI 입구 바이너리 먼저 빌드(CLI 경로 실험 필수 — 형제 위치에 놓이게)
//! cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send
//! # 2) 하네스 실행
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke                 # 기본(both, MCP)
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --priming prompts/agent-priming-cli.md --model sonnet
//! cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --cli-only    # provision 강제 비-MCP(false path 전체)
//! ```
//! CLI 입구가 필요한 프라이밍(본문이 engram-send/ENGRAM_SEND_EXE 를 언급 — 명시 경로 무관)인데 `engram-send`
//!   가 형제에 없으면, 하네스는 normal negative 가 아니라 **SETUP-SKIP**(engram-send not built) 라벨로 요란히
//!   알리고 종료한다 — 인프라 부재를 "B 가 안 보냄" 으로 오귀속하지 않는다. 판정은 셀렉터·basename 이 아니라
//!   **해석된 프라이밍 파일 본문(content)** 이라 명시 경로 override 와 CLI-지시 프라이밍까지 모두 잡힌다(ADR-0094).
//!
//! ## 핵심 불변식(ADR-0092/0086/0088)
//! - **required-features = ["test-harness"]** — 운영/릴리즈 빌드는 이 bin 을 컴파일하지 않는다.
//! - **프라이밍은 실물 파일에서**(ADR-0092) — 하드코딩 금지. 여기선 케이스→경로 매핑만 하고 `ENGRAM_PRIMING_FILE`
//!   env 로 FilePrimingProvider 에 넘긴다(두 에이전트가 같은 변형을 받게 provider 생성 **전에** set).
//!   ★`--cli-only` 예외★: 그 모드는 override 를 세우지 않고 `ENGRAM_FORCE_CLI_ONLY_SEND` 만 세운다 —
//!   provision 이 CliOnly 파일을 스스로 고르는 걸 관측한다(ADR-0099 FIX 3).
//! - **from 은 토큰 파생**(ADR-0086) — 씨앗 A→B 의 from = A 의 실 발급 신원(BoundIdentity), 본문 문자열 아님.
//! - **B 의 답신은 실 입구로만**(하네스가 handle_send 를 대신 부르지 않는다) — 이게 이 하네스의 핵심 새 검증.
//! - **배달 관측 = ADR-0088 in-proc 싱크** — registry 에 `DeliveryObserver` 를 설치해 relay 레코드를 회수한다
//!   (detached 데몬 로그 스크레이핑 금지). registry 에 read accessor 를 추가하지 않고 이 싱크로만 회수한다.
//! - **결과 3분류(FIX round-2 #2/#4/#5)** — ① **valid negative**(setup 성공했으나 B 가 안 보냄) = 구조화
//!   결과 출력 후 exit 0(유효한 실험 결과). ② **SETUP-SKIP**(exit 1) = 케이스가 요구하는 인프라 부재
//!   (CLI-지시 프라이밍인데 engram-send 미빌드 — 판정은 셀렉터·basename 이 아니라 **해석된 프라이밍 파일 본문**)
//!   — normal negative 로 오귀속 금지. ③ **SETUP-FAIL**(exit 1) = 준비 단계 실패(A/B 출력 구독 실패 /
//!   B 원과제 턴 실패 / A·B process death / 씨앗 ACK 에러 / priming 파일 부재). valid negative 는 setup 이
//!   온전히 성공했고 **A·B 가 모두 살아 있을 때만** 보고한다(A 死 → B 답신이 도달할 대상 없음).
//! - **skip_no_claude loud-skip** — claude 부재/인증 실패면 요란하게 스킵(silent skip 금지).
// ADR-0092

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::PresetRegistry;
use engram_dashboard_core::agent::profile::{
    AgentCommand, AgentProfile, ClaudeOutputFormat, ProfileRegistry, SpawnMode,
};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::ingress::{
    handle_send, ControlCommand, DeliveryObservation, DeliveryObserver, Entrance,
};
use engram_dashboard_daemon::control::mcp_server::{start_mcp_server, ManagerSlot};
use engram_dashboard_daemon::control::priming::{FilePrimingProvider, PrimingProvider};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;

/// 스폰 후 목록 등장 대기.
const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
/// 턴 종료(MessageDone) 대기 상한.
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);
/// B 답신(outbound relay) 대기 상한 — 초과 시 NEGATIVE(B did not send) 결과.
const REPLY_WAIT_CAP: Duration = Duration::from_secs(180);

/// A(발신자 팀원)의 표시 이름 — B 가 봉투에서 배워 `to=alice` 로 답신한다.
const NAME_A: &str = "alice";
/// B(수신·답신) 표시 이름.
const NAME_B: &str = "bob";

/// B 원과제(일하는 팀원 맥락) — auth 모듈 작업 중. 자연스러운 협업 셋업.
const TASK_PROMPT_B: &str =
    "You are currently working on the auth module (login/session). When you're ready to start, reply in one line.";

/// ★씨앗 A→B(ADR-0092 — 자연 팀원 질문, 기계적 "툴 X 써라" 아님)★: A 가 B 에게 진행 상황을 묻는
///   평범한 협업 질문 → 답을 A 에게 돌려주는 게 자연스러운 반응이 되도록 만든다. 발신 방법(툴/CLI)은
///   본문이 아니라 **프라이밍 변형**이 가르친다(C0/기본 = 프로덕션 both-teaching `prompts/agent-priming.md`).
const SEED_A_TO_B: &str =
    "Can you share the status of the auth module? If you're stuck anywhere on the login path, tell me what you need too.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

/// ★loud skip(priming_smoke 이식)★: claude 스폰 불가면 요란하게 스킵(exit 0 이되 SKIPPED 라벨을
///   stdout+stderr 에 남긴다 — silent skip 금지).
fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [roundtrip-smoke]: {reason} — A→B→A 왕복 실측 불가(claude 부재/인증).");
    println!("{line}");
    eprintln!("{line}");
    0
}

/// ★SETUP-SKIP(FIX round-2 #2)★: 선택 케이스가 요구하는 인프라(예: CLI 입구용 engram-send)가 없어
///   실험을 유효하게 돌릴 수 없을 때. **normal negative 와 구분되는** 라벨로 요란히 알린다 — 인프라
///   부재를 "B 가 안 보냄" 으로 오귀속하지 않는다. exit 1(설정 미비는 실험 결과가 아니라 실행 조건 미충족).
fn setup_skip(reason: &str) -> i32 {
    let line = format!("SETUP-SKIP [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

/// ★SETUP-FAIL(FIX round-2 #4)★: 스폰 후 실험 준비(B 원과제 턴 / 씨앗 ACK / B 생존) 중 하나가 진짜로
///   실패했을 때. valid negative("B did not send")와 **구분되는** 라벨로 알린다 — 유효 negative 는 setup
///   이 온전히 성공했을 때만 보고한다. exit 1(실험 결과가 아니라 setup 실패).
fn setup_fail(reason: &str) -> i32 {
    let line = format!("SETUP-FAIL [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

/// ★프라이밍 본문이 CLI 발신 경로를 지시하는가(순수·단위테스트 대상)★: 텍스트가 `engram-send` 또는
///   `ENGRAM_SEND_EXE` 를 언급하면 CLI 입구(engram-send)로 보내라는 프라이밍이다. 둘 중 하나만 있어도 true.
///   ★대소문자 무시(FIX)★: 본문 산문이 `ENGRAM-SEND`/`Engram-Send` 처럼 대소문자를 섞어 써도 잡아야 한다 —
///   놓치면 false negative(CLI 지시인데 미검출) → 인프라 부재를 정상 negative 로 오귀속. 본문을 한 번
///   lowercase 로 복사(단일 할당)해 소문자 리터럴과 대조한다.
///   ★basename 이 아니라 본문(content)인 이유★: 이전 판본은 하드코딩된 basename 리스트
///   (`agent-priming-send-cli.md`/`-send-both.md`)만 봤다. 그런 리스트는 rot 한다 — 새 CLI-지시 프라이밍
///   (v3-en-cli 등)이 리스트에서 누락돼 가드가 조용히 우회됐고, engram-send 부재(인프라 부재)가 SETUP-SKIP
///   대신 정상 negative(B_SENT=false)로 오귀속됐다. 그래서 파일명이 아니라 실제 본문을 진실의 출처로 본다 —
///   어느 프라이밍이든 CLI 발신을 지시하면 basename 과 무관하게 잡힌다.
///   ★의도적으로 보수적(부정문 false positive 는 수용)★: "engram-send 를 쓰지 마라" 같은 부정문도 substring
///   존재만으로 true → 헛된 SETUP-SKIP 이 될 수 있다. 그러나 SETUP-SKIP 은 요란한 exit-1 로, 틀릴 수 있는
///   데이터 발화를 거부하는 안전한 방향이다(실 프라이밍에 그런 부정문은 없다). 부정 파싱은 넣지 않는다 —
///   substring 존재 ⇒ CLI-지시로 취급, 헛된 skip 이 안전한 쪽.
fn priming_text_directs_cli(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("engram-send") || lower.contains("engram_send_exe")
}

/// ★--cli-only 가 상속된 ENGRAM_PRIMING_FILE override 와 충돌하는가(순수·단위테스트 대상, ADR-0099)★:
///   cli-only 모드는 provision 이 CliOnly 파일을 스스로 고르는 걸 관측하는 게 목적이라, 부모 env 에 미리
///   깔린 비어 있지 않은 override 는 그 auto-select 를 덮어써 관측을 무의미하게 만든다 → 충돌(true)로 본다.
///   `--priming` co-pass 거부와 대칭인 순수 판정자다. cli_only=false 면 env 값과 무관하게 충돌 아님(false)
///   — 운영/일반 모드는 override 를 정당히 쓴다. env 값이 비어 있으면(미설정 취급) 충돌 아님.
fn cli_only_env_override_conflicts(cli_only: bool, env_value: Option<&std::ffi::OsStr>) -> bool {
    cli_only && matches!(env_value, Some(v) if !v.is_empty())
}

/// ★--cli-only 성공 판정(순수·단위테스트 대상, ADR-0099)★: cli-only 모드에서 이 실측이 **성공(pass)** 인가.
///   이 모드는 provision 을 비-MCP 로 강제해 false path 전체가 정합하게 도는지를 실측하는 게 목적이므로,
///   B 가 실제로 발신했고(b_sent) 그 입구가 반드시 `cli` 여야만 성공이다 — 아무것도 안 보낸(b_sent=false,
///   entrance="none") 경우는 이 모드에선 **실패**로 본다(일반 모드의 valid-negative 와 다르다: 강제 false
///   path 가 도는 걸 못 봤으니 실측 목적 미달). entrance="mcp"(강제 seam 이 MCP 를 못 지움)는 앞선
///   SETUP-FAIL 이 이미 잡지만, 순수 판정자 수준에서도 cli 아닌 건 전부 실패로 매핑해 이중 안전망을 둔다.
fn cli_only_run_passed(b_sent: bool, entrance_label: &str) -> bool {
    b_sent && entrance_label == "cli"
}

/// CLI 인자 파싱 결과(순수) — priming 셀렉터 + 모델. `run` 이 이걸로 env·스폰을 배선한다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    /// `--priming` 값(프라이밍 파일 경로 — 절대/상대, 또는 편의 별칭 `C0`). 미지정이면 None(= 기본 both
    ///   프라이밍 `prompts/agent-priming.md`, `C0` 별칭과 동일). C1~C3 는 ADR-0099 로 별칭이 제거돼 이제 그냥
    ///   파일 경로로 해석된다(특수 매핑 없음 — 부재로 걸림).
    priming: Option<String>,
    /// `--model` 값(기본 sonnet).
    model: String,
    /// `--disallow-mcp` 플래그(ADR-0094 CLI-only 측정): 켜지면 `ENGRAM_DISALLOW_MCP_SEND` env 를 세워
    ///   두 에이전트가 MCP send_message grant **없이** 스폰 → engram-send CLI 로만 발신하게 강제한다.
    ///   test-only 측정 노브(운영 스위치 아님). 미지정이면 오늘 동작(MCP grant 포함).
    disallow_mcp: bool,
    /// `--cli-only` 플래그(ADR-0099 FIX 3): 켜지면 `ENGRAM_FORCE_CLI_ONLY_SEND` env 를 세워 provision 이
    ///   실 claude 스폰을 **비-MCP 백엔드로 강제**한다 → false path 전체(no mcp-config + CliOnly 프라이밍 +
    ///   [Cli] grant)가 돈다. ★`--disallow-mcp` 와 다른 점★: 후자는 MCP grant 만 빼고 MCP 서버는 여전히
    ///   부착·both-teaching 프라이밍이라 물리/교육 채널이 갈린다(측정용 불일치). `--cli-only` 는 provision
    ///   자체를 CLI-only 로 정렬해 정합 불변식을 보존한 채 false path 를 실측한다. ★이 모드는 `--priming`
    ///   override 를 주지 않아야 한다★ — provision 이 자동으로 `prompts/agent-priming-cli.md` 를 고르는 걸
    ///   보는 게 목적이다(entrance=cli 기대). test-only 노브(운영 스위치 아님).
    cli_only: bool,
}

/// 배달 관측 싱크(ADR-0088) — relay 레코드를 스레드 안전 Vec 에 모은다. 하네스가 registry 에 설치하고
///   나중에 from=B·to=A 레코드를 조회한다. registry 는 read accessor 를 노출하지 않으므로(write-only
///   observer 슬롯) 이 싱크가 회수 경로다.
struct CapturingObserver {
    records: Mutex<Vec<DeliveryObservation>>,
}

impl CapturingObserver {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
    /// 지금까지 관측된 레코드 총수(도착 순서 = Vec push 순서). 씨앗 주입 **직전**에 이 값을 baseline 으로
    ///   잡아, 그 이후에 도착한 레코드만 B 의 답신 후보로 본다(FIX round-2 #1 — pre-seed 오탐 차단).
    fn record_count(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// baseline **이후**에 도착한 레코드 중 from=`from_id`·to=`to_id` 인 첫 배달 스냅샷(있으면).
    ///   ★왜 baseline 절단인가(FIX round-2 #1)★: observer 는 B 의 원과제 턴 **전에** 설치된다. 만약 B 가
    ///   task-establishing 턴에서 A 에게 메시지를 하나 흘리면 그 pre-seed 레코드가 "답신" 으로 오인돼
    ///   거짓 B_SENT=true 를 내 실험을 오염시킨다. 그래서 씨앗 주입 직전 record_count 를 baseline 으로 잡고
    ///   `records[baseline..]` 만 훑어 씨앗에 인과적으로 뒤따르는 outbound 만 답신으로 본다.
    fn find_delivery_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Option<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)?
            .iter()
            .find(|r| r.from.agent_id == from_id && r.to_id == to_id)
            .cloned()
    }
}

impl DeliveryObserver for CapturingObserver {
    fn observe(&self, obs: DeliveryObservation) {
        self.records.lock().unwrap().push(obs);
    }
}

async fn run() -> i32 {
    let args = parse_args(std::env::args().skip(1));

    let repo_root = repo_root_from_manifest();
    let priming_selector = args.priming.clone();
    // ★--cli-only 는 priming override 를 주지 않아야 한다(ADR-0099 FIX 3)★: 이 모드의 목적은 provision 이
    //   자동으로 `prompts/agent-priming-cli.md`(CliOnly 변형)를 고르는 걸 보는 것이다. 그래서 여기서는
    //   ENGRAM_PRIMING_FILE override 를 세우지 않고, 보고·CLI-요구 판정용으로 그 CLI-only 운영 파일을
    //   effective priming 으로 해석만 한다. `--priming` 을 함께 주면 목적(auto-select 관측)과 충돌하므로
    //   fail-fast 한다(오해 방지).
    if args.cli_only && priming_selector.is_some() {
        return setup_fail(
            "--cli-only 는 --priming override 와 함께 쓸 수 없다 — 이 모드는 provision 이 자동으로 prompts/agent-priming-cli.md 를 고르는 걸 관측하는 게 목적이다(override 를 주면 그 관측이 무의미)",
        );
    }
    // ★--cli-only 는 **상속된** ENGRAM_PRIMING_FILE 도 거부한다(ADR-0099)★: 부모 env 에 이 override 가 미리
    //   깔려 있으면 provider(priming.rs)가 그걸 최우선으로 읽어 provision 의 CliOnly auto-select 를 조용히
    //   덮어쓴다 — `--priming` co-pass 거부와 같은 구멍이 env 로 들어온다. **조용히 clear 하지 않는다**
    //   (operator 가 일부러 세운 값일 수 있어 지우면 숨은 의도 파괴) — 어느 값이든(비어 있지 않으면) 그 이름을
    //   박아 SETUP-FAIL 로 요란히 거부하고 operator 가 직접 걷어내게 한다. co-pass 거부와 대칭이다.
    if cli_only_env_override_conflicts(
        args.cli_only,
        std::env::var_os("ENGRAM_PRIMING_FILE").as_deref(),
    ) {
        return setup_fail(
            "--cli-only 인데 부모 env 에 ENGRAM_PRIMING_FILE 이 설정돼 있다 — 이 override 가 provision 의 CliOnly auto-select 를 덮어써 관측을 무의미하게 만든다. 조용히 지우지 않으니(숨은 의도 파괴 방지) 실행 전에 직접 unset 하라",
        );
    }
    // effective priming 경로: cli-only 면 CliOnly 운영 파일(provision 이 auto-select 할 그 파일), 아니면
    //   셀렉터 해석 결과. 두 경우 모두 repo 루트 기준 절대화.
    let priming_selector_for_resolve = if args.cli_only {
        Some("prompts/agent-priming-cli.md")
    } else {
        priming_selector.as_deref()
    };
    let resolved_priming = match resolve_priming_path(priming_selector_for_resolve, &repo_root) {
        Some(p) => p,
        None => {
            // 절대화조차 못 함(비정상 셀렉터) — 프라이밍은 실험 필수라 fail-fast.
            return setup_fail(&format!(
                "priming 셀렉터({priming_selector:?})를 절대경로로 못 풂 — 실험 불가"
            ));
        }
    };
    // ★존재 검사 fail-fast(FIX round-2 #5)★: `FilePrimingProvider` 는 존재하지 않는 override 를 조용히
    //   버리고 UNPRIMED 로 스폰한다. 그러면 라벨은 "priming X 로 primed" 라 주장하지만 실제론 unprimed —
    //   케이스가 거짓말한다. 프라이밍은 이 실험의 본질이므로, 실제로 in-effect 가 아닌 경로는 절대 진행·
    //   출력하지 않는다. 여기서 확인해 없으면 SETUP-FAIL.
    if !resolved_priming.is_file() {
        return setup_fail(&format!(
            "priming 파일 없음: {} (case={:?}) — 존재하지 않는 override 는 UNPRIMED 스폰으로 이어져 케이스 라벨을 거짓으로 만든다",
            resolved_priming.display(),
            priming_selector
        ));
    }
    // ★프라이밍 본문 단일 읽기 + fail-closed(FIX)★: 여기서 딱 한 번 읽어 아래 CLI-요구 가드가 재사용한다.
    //   이전 판본은 존재 검사(위)와 가드에서 파일을 두 번 만졌고(TOCTOU 창), 가드 쪽은
    //   `read_to_string(...).unwrap_or(false)` 라 읽기 실패(공유 위반·권한·검사 후 삭제/교체·비-UTF-8)를
    //   전부 "CLI 요구 아님" 으로 삼켜 헛된 정상 negative 를 낼 수 있었다. 프라이밍 파일은 실험의 본질이므로
    //   읽을 수 없으면 조용히 진행하지 않고 SETUP-FAIL(exit 1). is_file 통과 후 여기서 즉시 읽어 그 창을 좁힌다.
    let priming_content = match std::fs::read_to_string(&resolved_priming) {
        Ok(c) => c,
        Err(e) => {
            return setup_fail(&format!(
                "priming 파일 읽기 실패: {} (case={:?}): {e} — 프라이밍은 실험 필수라 읽을 수 없으면 진행 불가",
                resolved_priming.display(),
                priming_selector
            ));
        }
    };
    // ★env 로 넘겨 FilePrimingProvider 생성 전에 set★: provision 마다 priming_file() 이 이 env 를
    //   최우선 override 로 읽어 두 에이전트(A·B) 모두 같은 변형을 받는다.
    //   ★--cli-only 예외(ADR-0099 FIX 3)★: 이 모드는 override 를 **세우지 않는다** — provision 이 강제된
    //     비-MCP 분기에서 CliOnly 변형(prompts/agent-priming-cli.md)을 스스로 고르는 걸 관측하는 게 목적이다.
    //     override 를 세우면 그 auto-select 를 우회하므로 일부러 뺀다.
    if !args.cli_only {
        std::env::set_var("ENGRAM_PRIMING_FILE", &resolved_priming);
    }
    eprintln!(
        "[roundtrip] priming = {} (case={:?}, cli_only={})",
        resolved_priming.display(),
        priming_selector,
        args.cli_only
    );
    // ★ADR-0094 CLI-only 측정 seam★: `--disallow-mcp` 가 켜지면 provision 전에 env 를 세워, 두 에이전트가
    //   MCP send_message grant **없이** 스폰돼 engram-send CLI 로만 발신하게 강제한다. build_grants 가 이
    //   env 를 읽는다(control/mod.rs). 프라이밍 env 와 같은 지점(provider·manager 배선 전)에 세워야 두
    //   에이전트 모두 같은 grant 셋으로 provision 된다. (CLI 입구 활성 = send_exe 존재는 아래에서 가드.)
    if args.disallow_mcp {
        std::env::set_var("ENGRAM_DISALLOW_MCP_SEND", "1");
        eprintln!("[roundtrip] --disallow-mcp → MCP send grant 제거(CLI-only 측정, ENGRAM_DISALLOW_MCP_SEND=1)");
    }
    // ★ADR-0099 FIX 3 CLI-only 강제 seam★: `--cli-only` 가 켜지면 provision 전에 env 를 세워, provision 이
    //   실 claude 스폰을 **비-MCP 로 강제**한다 → false path 전체(no mcp-config + CliOnly 프라이밍 + [Cli]
    //   grant)가 돈다. control/mod.rs::provision 이 이 env 를 분기 맨 위에서 읽어 effective flag 를 false 로
    //   덮는다. --disallow-mcp 와 달리 물리/교육 채널이 정합(둘 다 CLI)이라 실 claude 를 비-MCP 백엔드처럼
    //   굴려 false 분기를 실측한다(CLI 입구 활성 = send_exe 필수 — 아래에서 가드).
    if args.cli_only {
        std::env::set_var("ENGRAM_FORCE_CLI_ONLY_SEND", "1");
        eprintln!("[roundtrip] --cli-only → provision 을 비-MCP 로 강제(false path 전체, ENGRAM_FORCE_CLI_ONLY_SEND=1); entrance=cli 기대");
    }

    // 배선(priming_smoke 미러) — 실 FilePrimingProvider·MCP 서버·AgentManager.
    let registry = Arc::new(ControlRegistry::new());
    // ADR-0088: 배달 관측 싱크 설치 — B→A outbound relay 를 회수한다(로그 스크레이핑 금지).
    let observer = Arc::new(CapturingObserver::new());
    registry.set_delivery_observer(observer.clone());

    let slot = Arc::new(ManagerSlot::new());
    let handle = match start_mcp_server(registry.clone(), slot.clone()).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[roundtrip] MCP 서버 기동 실패: {e}");
            return 1;
        }
    };
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-roundtrip-{}", AgentId::new_v4()));
    let ws_a = std::env::temp_dir().join(format!("engram-roundtrip-ws-a-{}", AgentId::new_v4()));
    let ws_b = std::env::temp_dir().join(format!("engram-roundtrip-ws-b-{}", AgentId::new_v4()));
    let _ = std::fs::create_dir_all(&ws_a);
    let _ = std::fs::create_dir_all(&ws_b);

    // ★send_exe 배선(CLI 입구 활성화 — CLI-지시 프라이밍/`--cli-only`/`--disallow-mcp` 에 필수)★: engram-send 는 데몬 exe 형제로 배포된다. 이
    //   하네스는 cargo 가 만든 target 디렉토리(현재 exe 형제)에서 engram-send 를 찾아 endpoint 에 싣는다.
    //   못 찾으면 None(CLI 입구 비활성 — MCP 만).
    let send_exe = sibling_send_exe();
    match &send_exe {
        Some(p) => eprintln!("[roundtrip] engram-send = {}", p.display()),
        None => eprintln!("[roundtrip] engram-send 형제 바이너리 없음 — CLI 입구 비활성(MCP 만)."),
    }
    // ★CLI 요구 프라이밍인데 engram-send 부재 = SETUP-SKIP(ADR-0094)★: CLI 발신을 지시하는 프라이밍은
    //   B 가 CLI 입구로 보내도록 지시한다. send_exe 가 None 이면 B 는 그 경로로 물리적으로 못 보내므로,
    //   결과 B_SENT=false 는 "B 가 안 보내기로 함"(정상 negative)이 아니라 인프라 부재다. 판정은 셀렉터·
    //   basename 이 아니라 **해석된 프라이밍 파일 본문**으로 한다 — 명시 경로 override 도, basename 리스트에서
    //   누락되던 새 CLI-지시 프라이밍도 잡힌다. 위에서 단 한 번 읽어 둔 `priming_content` 를 순수 판정자
    //   `priming_text_directs_cli` 에 넘긴다(재읽기·TOCTOU 없음). 실 claude 2개를 스폰하기 **전에** 요란히
    //   SETUP-SKIP 하고 종료한다 — 헛된 스폰·오귀속 둘 다 막는다.
    if priming_text_directs_cli(&priming_content) && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(&format!(
            "engram-send not built, CLI inlet unavailable (case={:?} requires the CLI send path). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
            priming_selector
        ));
    }
    // ★--disallow-mcp 는 CLI 입구가 반드시 살아 있어야 한다(ADR-0094)★: MCP send grant 를 빼는데 CLI grant
    //   마저 없으면(send_exe=None) 두 에이전트는 발신 경로가 **하나도** 없어, B_SENT=false 는 정상 negative
    //   가 아니라 인프라 부재다. 위 CLI-요구 프라이밍 스킵과 같은 이유로 스폰 **전에** 요란히 SETUP-SKIP.
    if args.disallow_mcp && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--disallow-mcp requires the CLI inlet (engram-send) but it is not built — MCP grant removed AND no CLI grant means agents have no send path. 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
        );
    }
    // ★--cli-only 는 CLI 입구가 반드시 살아 있어야 한다(ADR-0099 FIX 3)★: 이 모드는 provision 을 비-MCP 로
    //   강제하므로 MCP 입구가 물리적으로 없다 — send_exe 마저 없으면 provision 이 fail-closed edge(Err)로
    //   스폰을 막는다(control/mod.rs). 그걸 SETUP-FAIL 로 늦게 만나기 전에 스폰 **전에** 요란히 SETUP-SKIP.
    if args.cli_only && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--cli-only requires the CLI inlet (engram-send) but it is not built — forced non-MCP spawn has no MCP inlet, and no CLI grant means agents have no send path (provision would fail-closed). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram-send` 로 형제 위치에 빌드하라",
        );
    }

    let priming_provider: Arc<dyn PrimingProvider> = Arc::new(FilePrimingProvider::new(repo_root));
    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        data_dir.clone(),
        send_exe,
        priming_provider,
    ));
    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    let profile_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-prof-{}", AgentId::new_v4()));
    let preset_dir =
        std::env::temp_dir().join(format!("engram-roundtrip-preset-{}", AgentId::new_v4()));
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        profile_dir.clone(),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        preset_dir.clone(),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());

    // ── A·B 스폰(둘 다 실 primed claude, stream-json, Fresh) ─────────────────────────
    // A 는 이름 alice(B 가 봉투에서 배워 to=alice 로 답신), B 는 bob.
    let agent_a = match spawn_named(&manager, NAME_A, &args.model, &ws_a) {
        Some(a) => a,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("A 스폰/등장 실패");
        }
    };
    let agent_b = match spawn_named(&manager, NAME_B, &args.model, &ws_b) {
        Some(b) => b,
        None => {
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[agent_a.id], &dirs).await;
            handle.shutdown().await;
            return skip_no_claude("B 스폰/등장 실패");
        }
    };
    eprintln!(
        "[roundtrip] spawned A(alice)={} B(bob)={} model={}",
        agent_a.id, agent_b.id, args.model
    );

    // A·B 각각에 출력 관측 sink 부착.
    let obs_a = Arc::new(TurnObserver::new());
    let obs_b = Arc::new(TurnObserver::new());
    let sink_a = manager.subscribe(agent_a.id, obs_a.clone()).ok();
    let sink_b = manager.subscribe(agent_b.id, obs_b.clone()).ok();

    // ★setup-failure 시 공통 정리(FIX round-2 #4)★: 아래 setup 단계에서 hard-fail 하면 이 클로저로 구독
    //   해제·kill·디렉토리 정리·MCP 종료를 하고 SETUP-FAIL 을 낸다(valid negative 와 구분).
    macro_rules! fail_setup {
        ($reason:expr) => {{
            if let Some(id) = sink_a {
                let _ = manager.unsubscribe(agent_a.id, id);
            }
            if let Some(id) = sink_b {
                let _ = manager.unsubscribe(agent_b.id, id);
            }
            let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
            cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
            handle.shutdown().await;
            return setup_fail($reason);
        }};
    }

    // ★A 구독 실패 = SETUP-FAIL(FIX round-2 #4)★: A 의 `TurnObserver` 를 못 붙이면(sink_a=None) A 가
    //   답신을 처리하며 낸 텍스트(정성 관측)를 아예 볼 수 없다 — 그 상태의 정성 결과는 무의미하므로 valid
    //   negative 로 보고하면 안 된다. B 구독 실패도 같은 이유(B 턴 관측 불가 → 원과제 setup 판정 불가).
    if sink_a.is_none() {
        fail_setup!("A 출력 구독 실패(sink_a=None) — A 턴 관측 불가, 정성 결과 무의미(setup 실패)");
    }
    if sink_b.is_none() {
        fail_setup!("B 출력 구독 실패(sink_b=None) — B 턴 관측 불가, setup 판정 불가(setup 실패)");
    }

    // ── 1) B 원과제 턴(일하는 팀원 맥락) ────────────────────────────────────────────
    // ★turn 실패 = setup 실패(FIX round-2 #4)★: 이전엔 warn 후 계속했다 — 그러면 "일하는 팀원 맥락" 이
    //   서지 않은 채 B_SENT=false 를 정상 negative 로 보고해 setup 실패를 실험 결과로 오인한다. B 가 원과제를
    //   수용(턴 종료)하지 못하거나 그 사이 죽으면 valid negative 가 아니라 SETUP-FAIL 이다.
    if !send_and_wait(&manager, agent_b.id, &obs_b, TASK_PROMPT_B) {
        if !is_agent_alive(&manager, agent_b.id) {
            fail_setup!("B 가 원과제 턴 도중 종료됨(process death) — 팀원 맥락 setup 실패");
        }
        fail_setup!(
            "B 원과제 턴이 cap 내 종료 신호 없음 — 팀원 맥락 setup 실패(valid negative 아님)"
        );
    }
    eprintln!(
        "[roundtrip] --- B task turn ---\n{}\n--- end ---",
        obs_b.response_text().trim()
    );
    // 씨앗 주입 직전 B 생존 재확인 — task 턴은 끝났지만 그 뒤 죽었을 수 있다.
    if !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B 가 씨앗 주입 전 종료됨 — setup 실패");
    }

    // ── 2) 씨앗 A→B(실 control 경로, from = A 의 실 발급 신원) ────────────────────────
    // ★from = 토큰 파생(ADR-0086)★: A 는 Fresh 스폰이라 epoch 0 — provision 이 그 (id,0)에 토큰을 이미
    //   발급했다(registry 에 산 신원). 본문 문자열이 아니라 이 BoundIdentity 가 발신자다.
    let from_a = BoundIdentity {
        agent_id: agent_a.id,
        epoch: 0,
    };
    // A 의 답신 관측 baseline 을 씨앗 주입 **전에** 잡는다(B 답신이 A 턴을 밀어 올리는 걸 본다).
    obs_a.begin_turn();
    let baseline_a = obs_a.done_snapshot();
    // ★B→A relay baseline(FIX round-2 #1)★: 씨앗 주입 **직전**에 관측 레코드 수를 잡는다. B 가 원과제 턴에서
    //   A 에게 흘린 pre-seed 레코드가 답신으로 오인되는 걸 막는다 — 이후 도착분만 답신 후보.
    let reply_baseline = observer.record_count();
    // ★진단(탐색)★: B 가 씨앗을 받고 자기 턴에 응답은 하는데 send 로 라우팅만 안 하는지 보려고 B 의 씨앗-후
    //   턴 텍스트를 캡처한다. begin_turn 은 text 만 비우고 done_count 는 누적이라, reply 대기 동안 B 턴이
    //   끝나면 response_text 에 씨앗-후 출력이 담긴다.
    obs_b.begin_turn();
    let baseline_b = obs_b.done_snapshot();

    let seed = ControlCommand {
        from: from_a,
        to: NAME_B.to_string(), // 이름으로 지목(alice→bob).
        body: SEED_A_TO_B.to_string(),
    };
    let ack = handle_send(&manager, &registry, Entrance::Cli, seed);
    eprintln!("[roundtrip] seed A→B ACK = {}", ack.to_json());
    // ★씨앗 ACK 에러 = setup 실패(FIX round-2 #4)★: ACK 가 error(수신자 미해석·write 실패 등)면 B 는 애초에
    //   씨앗을 못 받았다 — 그 뒤 B_SENT=false 는 "B 가 답 안 함" 이 아니라 씨앗 배달 실패다.
    if !ack.is_enqueued() {
        fail_setup!(&format!(
            "씨앗 A→B ACK 가 enqueued 아님(배달 실패): {}",
            ack.to_json()
        ));
    }

    // ── 3) B 의 답신을 **B 자신의 발신 경로**로 대기(하네스는 handle_send 를 부르지 않는다) ──────
    //    B(실 claude)가 MCP send_message 또는 engram-send CLI 를 스스로 호출 → 실 입구 → handle_send →
    //    wrap → A stdin. 그 relay 가 관측 싱크에 baseline 이후 from=B·to=A 레코드로 남는지 폴링한다.
    let reply_obs = wait_for_reply(
        &observer,
        reply_baseline,
        agent_b.id,
        agent_a.id,
        REPLY_WAIT_CAP,
    );
    let b_sent = reply_obs.is_some();
    // ★valid negative 게이트(FIX round-2 #4)★: B 가 안 보냈는데(reply_obs=None) 그 사이 **A 또는 B** 가
    //   죽었다면 그건 "B 가 안 보내기로 함"(정상 negative)이 아니라 process death setup 실패다.
    //   - B 사망: B 가 답신을 만들 주체를 잃음.
    //   - A 사망: A 가 죽으면(스폰 후 / 씨앗 ACK 후) B 의 답신이 A 에 도달할 대상이 없어 관측이 안 뜨고,
    //     B 는 살아 있어 기존 B-only 게이트는 이를 정상 negative 로 오분류한다 — 이게 원 blocker 와 같은
    //     방식으로 실험 데이터를 오염시킨다. 그래서 valid negative 판정 지점에서 A 생존도 함께 확인한다.
    //   A·B 모두 살아 있는데도 안 보낸 경우만 유효한 실험 negative 로 아래에서 보고한다.
    if !b_sent && !is_agent_alive(&manager, agent_b.id) {
        fail_setup!("B 가 답신 대기 중 종료됨(process death) — valid negative 아님(setup 실패)");
    }
    if !b_sent && !is_agent_alive(&manager, agent_a.id) {
        fail_setup!(
            "A 가 답신 대기 중 종료됨(process death) — B 답신이 도달할 대상 없음, valid negative 아님(setup 실패)"
        );
    }
    let entrance_label = match &reply_obs {
        Some(o) => entrance_str(o.entrance),
        None => "none",
    };
    // ★--cli-only 판정(ADR-0099 FIX 3)★: 이 모드는 provision 을 비-MCP 로 강제해 MCP 입구가 물리적으로
    //   없다 — B 가 보냈다면(b_sent) entrance 는 반드시 `cli` 여야 한다. `mcp` 가 관측되면 강제 seam 이
    //   실제로 MCP 를 제거하지 못한 것(배관 결함)이므로 SETUP-FAIL(setup 결함)로 요란히 알린다.
    //   ★entrance=none(B 미발신)은 여기서 안 잡고 끝의 엄격 VERDICT 가 FAIL(exit 1)로 처리한다★ —
    //   여기 SETUP-FAIL 은 "seam 배관 결함"(mcp 새어나옴) 전용이고, "강제 false path 미실증"(아무도 안 보냄)은
    //   결과 판정이라 최종 VERDICT 로 분리한다(라벨이 서로 다른 실패 원인을 섞지 않게).
    if args.cli_only && b_sent && entrance_label != "cli" {
        fail_setup!(&format!(
            "--cli-only 인데 B 가 entrance={entrance_label} 로 발신 — 강제 seam 이 MCP 입구를 제거 못 함(배관 결함, 정상 negative 아님)"
        ));
    }

    // ── 4) A 가 B 답신을 처리하며 낸 텍스트 대기(정성 관측) ───────────────────────────
    //    B 가 보냈으면 그 relay 가 A stdin 에 꽂혀 A 턴이 돈다. 남은 시간만큼 A 턴 종료를 기다린다.
    let a_responded = if b_sent {
        obs_a.wait_turn_end(baseline_a, TURN_WAIT_CAP)
    } else {
        // B 가 안 보냈으면 A 턴이 돌 이유가 없다 — 짧게만 확인(이미 REPLY_WAIT_CAP 동안 아무것도 없었음).
        obs_a.done_snapshot() > baseline_a
    };
    let a_response = obs_a.response_text();

    // ── 5) 구조화 stdout 마커(오케스트레이터 판정용) ────────────────────────────────
    // cli-only 모드는 셀렉터가 없으므로(override 금지) 전용 라벨을 단다 — 오케스트레이터가 이 실측이
    //   false-path(provision 강제 비-MCP) 임을 구분하게.
    let case_label = if args.cli_only {
        "CLI-ONLY(forced non-MCP)"
    } else {
        priming_selector.as_deref().unwrap_or("C0")
    };
    println!("\n===== ROUNDTRIP CASE={case_label} B_SENT={b_sent} ENTRANCE={entrance_label} =====");
    println!("[model] {}", args.model);
    // 존재 검사를 통과한 실제 in-effect 경로만 출력한다(FIX round-2 #5 — 거짓 라벨 금지).
    println!("[priming] {}", resolved_priming.display());
    println!("[seed A->B body] {SEED_A_TO_B}");
    println!("[B sent reply to A] {b_sent}");
    println!("[B chosen entrance] {entrance_label}");
    if let Some(o) = &reply_obs {
        // 봉투 배달 레코드는 body 텍스트를 담지 않는다(보안) — 바이트 수·msg_id 만.
        println!(
            "[B->A delivery] msg_id={} bytes={} to_epoch={:?}",
            o.msg_id, o.bytes_requested, o.to_epoch
        );
    }
    println!("[A responded within cap] {a_responded}");
    println!("[A full response text]\n{}", a_response.trim());
    // 진단: B 가 씨앗을 받고 자기 턴에 낸 응답(send 라우팅과 무관 — B_SENT=false 여도 여기 텍스트가 있으면
    //   "B 는 답했으나 send 로 안 보냄"이고, 비면 "B 가 씨앗에 반응 안 함").
    let b_turn_ended = obs_b.done_snapshot() > baseline_b;
    let b_seed_response = obs_b.response_text();
    println!("[B post-seed turn ended] {b_turn_ended}");
    println!("[B post-seed turn text]\n{}", b_seed_response.trim());
    println!("===== END ROUNDTRIP (orchestrator judges qualitatively) =====\n");

    // ── 정리 ──────────────────────────────────────────────────────────────────────
    if let Some(id) = sink_a {
        let _ = manager.unsubscribe(agent_a.id, id);
    }
    if let Some(id) = sink_b {
        let _ = manager.unsubscribe(agent_b.id, id);
    }
    let dirs = [&data_dir, &ws_a, &ws_b, &profile_dir, &preset_dir];
    cleanup(&manager, &[agent_a.id, agent_b.id], &dirs).await;
    handle.shutdown().await;
    // ★--cli-only 는 엄격 판정(ADR-0099)★: 이 모드는 provision 을 비-MCP 로 강제해 false path 전체가
    //   정합하게 도는지를 실측하는 게 목적이라, B 가 실제로 CLI 입구로 보냈을 때만(b_sent && entrance=cli)
    //   성공이다. 아무것도 안 보낸 경우(B_SENT=false/ENTRANCE=none)는 일반 모드의 valid-negative 와 달리
    //   **실패**로 본다(강제 false path 가 도는 걸 못 봤으니 목적 미달). 일반 모드는 종전대로 negative 도 exit 0.
    if args.cli_only {
        if cli_only_run_passed(b_sent, entrance_label) {
            println!("VERDICT [roundtrip-smoke --cli-only]: PASS — B 가 CLI 입구로 발신(b_sent=true, entrance=cli)");
            return 0;
        }
        let line = format!(
            "VERDICT [roundtrip-smoke --cli-only]: FAIL — 강제 false path 미실증(b_sent={b_sent}, entrance={entrance_label}); cli-only 는 b_sent=true AND entrance=cli 여야 pass"
        );
        println!("{line}");
        eprintln!("{line}");
        return 1;
    }
    // ★negative(B did not send)도 정상 exit 0★: 유효한 실험 결과지 하네스 실패가 아니다(ADR-0092).
    0
}

/// ★인자 파싱(순수·단위테스트 대상)★: `--priming <값>`·`--model <값>`·불리언 `--disallow-mcp`/`--cli-only`
///   를 인식한다. 미지정 model=sonnet, 미지정 priming=None(= 기본 both 프라이밍). 알 수 없는 토큰은 무시
///   (하네스라 관대). `iter` 로 받아 std::env 의존을 뺀다.
/// ★플래그를 값으로 삼키지 않는다(FIX round-2 #7)★: `--priming --model opus` 처럼 다음 토큰이 또 플래그
///   (`--` 로 시작)면 그건 값이 아니라 새 플래그다 — peek 해서 값으로 소비하지 않고 넘긴다(그 플래그는
///   다음 루프에서 제대로 처리, priming 은 미지정 유지). 이렇게 안 하면 `--model` 이 priming 값으로 먹혀
///   model 이 조용히 기본값에 남는다.
fn parse_args(iter: impl Iterator<Item = String>) -> Args {
    let mut priming = None;
    let mut model = "sonnet".to_string();
    // ADR-0094: `--disallow-mcp` 는 값 없는 불리언 플래그(존재 = 켜짐) — take_flag_value 로 다음 토큰을
    //   삼키지 않는다(그 자체로 완결).
    let mut disallow_mcp = false;
    // ADR-0099 FIX 3: `--cli-only` 도 값 없는 불리언 플래그(존재 = 켜짐).
    let mut cli_only = false;
    let mut it = iter.peekable();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--priming" => {
                if let Some(v) = take_flag_value(&mut it) {
                    priming = Some(v);
                }
            }
            "--model" => {
                if let Some(v) = take_flag_value(&mut it) {
                    model = v;
                }
            }
            "--disallow-mcp" => disallow_mcp = true,
            "--cli-only" => cli_only = true,
            _ => {}
        }
    }
    Args {
        priming,
        model,
        disallow_mcp,
        cli_only,
    }
}

/// 플래그 값 하나를 소비하되, 다음 토큰이 또 다른 플래그(`--`)면 소비하지 않는다(FIX round-2 #7).
///   반환 None = 값 없음(플래그가 값 없이 끝났거나 다음이 또 플래그) → 호출자는 기본값 유지.
fn take_flag_value<I: Iterator<Item = String>>(it: &mut std::iter::Peekable<I>) -> Option<String> {
    match it.peek() {
        Some(next) if next.starts_with("--") => None, // 다음이 플래그 → 값 아님(넘김, 소비 X).
        Some(_) => it.next(),                         // 정상 값 → 소비.
        None => None,                                 // 값 없이 끝.
    }
}

/// ★셀렉터→priming 파일 경로(순수·단위테스트 대상, ADR-0099)★: repo 루트 기준 경로로 매핑한다.
///   - C0(또는 None) → `prompts/agent-priming.md`(운영 A = both-teaching).
///   - 그 외 = **파일 경로로 간주**(절대면 그대로, 상대면 repo 루트 기준 join) — 명시 override. 운영 B
///     (`prompts/agent-priming-cli.md`)나 임시 실험 파일을 이 경로로 직접 지정한다.
/// 반환은 항상 절대경로(존재 검사는 하지 않는다 — FilePrimingProvider 가 최종 존재/CLI-안전 검사).
///   절대화조차 못 하면 None.
///   ※ 옛 C1~C3 실험 별칭은 ADR-0099 로 제거됐다(실험 변형 파일 정리 — git 이력 보존). C1~C3 문자열을
///     넘기면 이제 "그 이름의 파일 경로"로 해석돼 repo 루트 기준 join 되고(존재하지 않아 하류에서 None),
///     별도 특수 매핑은 없다.
fn resolve_priming_path(selector: Option<&str>, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel: &str = match selector {
        None | Some("C0") | Some("c0") => "prompts/agent-priming.md",
        Some(path) => {
            // 명시 경로 override. 절대면 그대로, 상대면 repo 루트 기준.
            let p = PathBuf::from(path);
            let joined = if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            };
            return joined.is_absolute().then_some(joined);
        }
    };
    let joined = repo_root.join(rel);
    joined.is_absolute().then_some(joined)
}

/// 어느 입구 라벨인가(관측 레코드 → 문자열). Entrance 는 daemon crate 내부 as_str 이 private 이라
///   여기서 매핑한다(하네스 표시 전용).
fn entrance_str(e: Entrance) -> &'static str {
    match e {
        Entrance::Mcp => "mcp",
        Entrance::Cli => "cli",
    }
}

/// 이 크레이트 매니페스트에서 두 단계 위로 올라간 repo 루트(`prompts/` 가 그 아래).
///   ★discovery(FIX round-2 #6)★: `priming_smoke.rs` 와 **같은** 컴파일타임 `CARGO_MANIFEST_DIR` 기반
///   방식이다(둘 다 동일 — 확인함). 운영 데몬의 exe-walk-up(`discovery::find_install_root`, ADR-0092:
///   WMI 스폰이라 cwd 불신)과는 다르지만, 이 실험 하네스는 항상 `cargo run` 으로 도는 컴파일타임 소스
///   트리 안이므로 MANIFEST_DIR 이 신뢰 가능하다(빌드된 bin 을 다른 곳으로 옮겨 실행하는 경로는 없다).
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent() // .../crates
        .and_then(|p| p.parent()) // .../engram-dashboard (repo 루트)
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

/// 현재 exe 형제에서 `engram-send`(Windows 는 .exe) 를 찾는다 — CLI 입구를 켜려면 필요. 못 찾으면 None
///   (CLI 입구 비활성, MCP 만). cargo run 시 exe 는 target/<profile>/ 아래라 engram-send 도 그 형제다.
fn sibling_send_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "engram-send.exe"
    } else {
        "engram-send"
    };
    let cand = dir.join(name);
    cand.is_file().then_some(cand)
}

/// 에이전트가 아직 살아 있나(비-terminal 상태) — setup 실패 vs valid negative 판별(FIX round-2 #4).
///   목록에서 사라졌거나 terminal(Exited/Failed/Killed)이면 false. Running/Exiting 은 alive.
fn is_agent_alive(manager: &Arc<AgentManager>, id: AgentId) -> bool {
    manager
        .list_agents()
        .iter()
        .find(|a| a.id == id)
        .map(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Exiting))
        .unwrap_or(false)
}

/// 이름 붙인 primed claude(stream-json, Fresh) 1개 스폰 + 목록 등장 대기. 실패/미등장이면 None.
fn spawn_named(
    manager: &Arc<AgentManager>,
    name: &str,
    model: &str,
    workspace: &std::path::Path,
) -> Option<AgentInfo> {
    // ★canonical name = display_name(ADR-0101 WYSIWYA)★: 라우팅·로스터·봉투 sender 가 쓰는 이름 =
    //   display_name ?? basename(session.cwd) 다(profile.name 은 더 이상 주소축 아님). 두 에이전트는
    //   같은 workspace cwd 를 공유해 basename 이 동일 → cwd 파생이면 alice/bob 이 같은 이름으로 충돌·
    //   오라우팅(bob 로 답신)한다. 그래서 이름을 display_name 에 심어 **결정적으로** 구분한다.
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec!["--model".to_string(), model.to_string()],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        workspace.to_path_buf(),
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    let info = manager.spawn_agent(&profile, SpawnMode::Fresh).ok()?;
    let deadline = Instant::now() + SPAWN_APPEAR_TIMEOUT;
    while Instant::now() < deadline {
        if manager.list_agents().iter().any(|a| a.id == info.id) {
            return Some(info);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    None
}

/// baseline **이후** 도착한 from=B·to=A outbound relay 레코드를 상한까지 폴링. 나타나면 그 레코드,
///   상한 초과면 None(negative — B 가 안 보냄). `baseline` = 씨앗 주입 직전 record_count(FIX round-2 #1
///   — pre-seed 오탐 차단). 폴링인 이유: relay 는 B 의 실 claude 판단에 달려 비결정적 지연을 가진다
///   (cv 신호원이 없어 짧은 sleep 폴링이 단순·충분).
fn wait_for_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer.find_delivery_after(baseline, from_b, to_a) {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 프롬프트를 유저 턴으로 보내고 이번 턴 종료(MessageDone)까지 대기. priming_smoke 와 동일.
fn send_and_wait(
    manager: &Arc<AgentManager>,
    id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
) -> bool {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    if manager.write_stdin_observed(id, prompt.as_bytes()).is_err() {
        return false;
    }
    obs.wait_turn_end(baseline, TURN_WAIT_CAP)
}

async fn cleanup(manager: &Arc<AgentManager>, agent_ids: &[AgentId], dirs: &[&PathBuf]) {
    for id in agent_ids {
        let _ = manager.kill_agent(*id);
    }
    if !agent_ids.is_empty() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !manager.list_agents().is_empty() {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// 턴 관측기 — MessageDone 카운트(턴 종료 신호) + TextDelta 누적(응답 텍스트).
///
/// ★lost-wakeup 방지(FIX round-2 #3)★: 이전 판본은 `done_count` 를 `AtomicU64` 로 두고 mutex 밖에서
///   증가·notify 했다. 그러면 waiter 가 [원자 predicate 체크] 와 [`wait_timeout` 등록] 사이에 완료 신호가
///   끼면 그 wakeup 을 잃고(cv 는 등록 전 notify 를 기억하지 않는다) 이미 끝난 턴을 상한(cap)까지 헛대기해
///   **거짓 타임아웃**(A 무응답/ B task 타임아웃)을 낸다. 그래서 표준 condvar 규율로 바꾼다:
///   predicate 상태(`done_count`)를 **cv 가 쓰는 바로 그 mutex 안**에 넣고, 발신·대기 모두 그 락을 잡은 채
///   갱신/재확인한다 → notify 는 락 해제 후 관측되므로 wakeup 손실이 원천적으로 없다.
///   `inner`(응답 텍스트)와 `done_count` 를 한 구조체(`State`)로 묶어 단일 mutex 로 보호한다.
struct TurnState {
    /// 이번 턴 누적 응답 텍스트.
    text: String,
    /// 관측된 MessageDone 누계(턴 종료 신호). cv predicate 의 단일 출처 — 이 mutex 로만 접근.
    done_count: u64,
}

struct TurnObserver {
    id: SinkId,
    state: Mutex<TurnState>,
    cv: Condvar,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            state: Mutex::new(TurnState {
                text: String::new(),
                done_count: 0,
            }),
            cv: Condvar::new(),
        }
    }
    fn begin_turn(&self) {
        self.state.lock().unwrap().text.clear();
    }
    fn done_snapshot(&self) -> u64 {
        self.state.lock().unwrap().done_count
    }
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.state.lock().unwrap();
        loop {
            // predicate 를 mutex 보유 중 재확인(표준 condvar 루프) — notify 는 이 락 안에서만 반영된다.
            if g.done_count > baseline {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (ng, _to) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }
    fn response_text(&self) -> String {
        self.state.lock().unwrap().text.clone()
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            return Ok(());
        };
        match ev {
            OutputEvent::TextDelta { text, .. } => {
                self.state.lock().unwrap().text.push_str(text);
            }
            OutputEvent::MessageDone { .. } => {
                // ★락 보유 중 상태 변경 후 notify(wakeup 손실 방지)★: predicate(done_count)를 cv 의 mutex
                //   안에서 올린다. guard 를 notify 후 drop 해도 되고 전에 drop 해도 되지만, 표준 규율대로
                //   보유 중 변경만 지키면 [체크↔등록] 갭에 낀 완료가 사라지지 않는다.
                let mut g = self.state.lock().unwrap();
                g.done_count += 1;
                drop(g);
                self.cv.notify_all();
            }
            _ => {}
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(s(&[]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
        assert!(!a.disallow_mcp, "기본은 MCP 허용(오늘 동작)");
        assert!(!a.cli_only, "기본은 cli-only 강제 없음(오늘 동작)");
    }

    #[test]
    fn parse_args_cli_only_flag_is_boolean() {
        // ★ADR-0099 FIX 3★: `--cli-only` 는 값 없는 불리언 플래그(존재 = 켜짐). 뒤 토큰(--model)을 값으로
        //   삼키지 않고, model 은 정상 파싱돼야 한다.
        let a = parse_args(s(&["--cli-only", "--model", "opus"]));
        assert!(a.cli_only, "--cli-only 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--cli-only 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_cli_only_absent_is_false() {
        // 플래그 미지정이면 오늘 동작(강제 없음) 유지 — 운영 회귀 0.
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
        assert!(!a.cli_only);
    }

    #[test]
    fn parse_args_disallow_mcp_flag_is_boolean() {
        // ★ADR-0094★: `--disallow-mcp` 는 값 없는 불리언 플래그(존재 = 켜짐). 뒤 토큰(--model)을 값으로
        //   삼키지 않고, model 은 정상 파싱돼야 한다.
        let a = parse_args(s(&["--disallow-mcp", "--model", "opus"]));
        assert!(a.disallow_mcp, "--disallow-mcp 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--disallow-mcp 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_disallow_mcp_absent_is_false() {
        // 플래그 미지정이면 오늘 동작(MCP 허용) 유지 — 운영 회귀 0.
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "haiku"]));
        assert!(!a.disallow_mcp);
    }

    #[test]
    fn parse_args_priming_and_model() {
        let a = parse_args(s(&["--priming", "some/priming.md", "--model", "opus"]));
        assert_eq!(a.priming.as_deref(), Some("some/priming.md"));
        assert_eq!(a.model, "opus");
    }

    #[test]
    fn parse_args_order_independent_and_ignores_unknown() {
        let a = parse_args(s(&[
            "--model",
            "haiku",
            "junk",
            "--priming",
            "other/priming.md",
        ]));
        assert_eq!(a.priming.as_deref(), Some("other/priming.md"));
        assert_eq!(a.model, "haiku");
    }

    #[test]
    fn parse_args_flag_without_value_is_ignored() {
        // --priming 뒤에 값이 없으면 priming 은 None 유지(패닉 없이 관대).
        let a = parse_args(s(&["--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
    }

    #[test]
    fn parse_args_flag_does_not_consume_next_flag_as_value() {
        // ★FIX round-2 #7★: `--priming --model opus` — --priming 은 값이 없고(다음이 플래그), --model 은
        //   제대로 opus 로 파싱돼야 한다(이전엔 --model 이 priming 값으로 먹혀 model 이 sonnet 에 남았다).
        let a = parse_args(s(&["--priming", "--model", "opus"]));
        assert_eq!(
            a.priming, None,
            "다음 토큰이 플래그면 priming 값으로 삼키지 않는다"
        );
        assert_eq!(a.model, "opus", "--model 은 정상 파싱돼야");
    }

    #[test]
    fn parse_args_trailing_flag_flags_both_ignored_cleanly() {
        // 둘 다 값 없이 끝나는 malformed — 패닉 없이 기본값 유지.
        let a = parse_args(s(&["--model", "--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(
            a.model, "sonnet",
            "--model 뒤가 플래그라 값 없음 → 기본 유지"
        );
    }

    #[test]
    fn resolve_case_c0_maps_to_current_priming() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(None, &root).expect("C0 경로");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md"),
            "C0 은 현행 priming: {got:?}"
        );
        // 명시 "C0" 셀렉터도 같은 경로.
        let got2 = resolve_priming_path(Some("C0"), &root).expect("C0 경로");
        assert_eq!(got, got2);
    }

    // ADR-0099: 옛 C1~C3 실험 별칭 매핑 테스트는 별칭 제거와 함께 삭제됐다. C1~C3 는 이제 파일 경로로
    //   해석돼 repo 루트 기준 join 될 뿐 특수 매핑이 없다(아래 명시 경로 override 테스트가 그 동작을 커버).

    #[test]
    fn resolve_explicit_absolute_path_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "C:\\custom\\my-priming.md"
        } else {
            "/custom/my-priming.md"
        };
        let got = resolve_priming_path(Some(abs), &root).expect("절대 override");
        assert_eq!(got, PathBuf::from(abs), "절대 경로는 그대로 통과");
    }

    #[test]
    fn resolve_explicit_relative_path_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_priming_path(Some("sub/custom.md"), &root).expect("상대 override");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/custom.md") || got.ends_with("sub\\custom.md"),
            "상대 경로는 repo 루트 기준 join: {got:?}"
        );
    }

    // ★ADR-0094★: CLI-요구 판정은 basename 리스트가 아니라 프라이밍 **본문(content)** 으로 한다 —
    //   `engram-send` 또는 `ENGRAM_SEND_EXE` 를 언급하면 CLI 발신 지시. basename 리스트는 rot 하므로
    //   (새 CLI-지시 프라이밍이 누락돼 가드 우회 → 인프라 부재 오귀속) 본문을 진실의 출처로 삼는다.
    #[test]
    fn priming_text_directs_cli_true_for_engram_send_mention() {
        // engram-send CLI 를 언급하는 본문 → true(ENGRAM_SEND_EXE 와 engram-send 둘 다 등장).
        let text = "To reply, run in your shell: `$ENGRAM_SEND_EXE --to alice --body ...`\n\
                    i.e. run the engram-send command with the recipient name.";
        assert!(
            priming_text_directs_cli(text),
            "engram-send 언급 → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_true_for_env_var_only() {
        // ENGRAM_SEND_EXE 만 있어도(engram-send 리터럴 없이) CLI 지시.
        let text = "Invoke the binary referenced by ENGRAM_SEND_EXE to deliver your message.";
        assert!(
            priming_text_directs_cli(text),
            "ENGRAM_SEND_EXE 언급 → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_false_for_mcp_only() {
        // MCP send_message 만 언급하고 CLI 경로는 없음 → false(CLI 없이도 유효한 실험).
        let text = "To reply, call the MCP tool `send_message` with the recipient and body.";
        assert!(
            !priming_text_directs_cli(text),
            "MCP send_message 만 → CLI 지시 아님"
        );
    }

    #[test]
    fn priming_text_directs_cli_false_for_empty() {
        // 빈 본문(발신 지시 없음) → false.
        assert!(!priming_text_directs_cli(""), "빈 본문 → CLI 지시 아님");
    }

    #[test]
    fn priming_text_directs_cli_case_insensitive() {
        // ★FIX★: 대소문자 무시 — 산문이 대문자/혼합으로 써도 CLI 지시로 잡아야 한다(놓치면 false negative).
        assert!(
            priming_text_directs_cli("Reply via ENGRAM-SEND right away."),
            "대문자 ENGRAM-SEND → CLI 지시"
        );
        assert!(
            priming_text_directs_cli("Use the Engram-Send helper to deliver."),
            "혼합 Engram-Send → CLI 지시"
        );
        assert!(
            priming_text_directs_cli("The var Engram_Send_Exe points to the binary."),
            "혼합 Engram_Send_Exe → CLI 지시"
        );
    }

    #[test]
    fn priming_text_directs_cli_negation_is_intentionally_true() {
        // ★수용된 false positive(문서화)★: "engram-send 를 쓰지 마라" 같은 부정문도 substring 존재만으로
        //   true → 헛된 SETUP-SKIP. 이는 의도된 보수적 방향이다(요란한 exit-1 로 틀릴 수 있는 발화 거부).
        //   실 프라이밍엔 그런 부정문이 없고, 부정 파싱은 넣지 않는다. 순수 레벨의 현 동작을 못박아 둔다.
        assert!(
            priming_text_directs_cli("Do NOT use engram-send; use MCP instead."),
            "부정문도 substring 존재로 true — 의도된 보수적 skip 방향"
        );
    }

    // ── ADR-0099: --cli-only 가 상속된 ENGRAM_PRIMING_FILE override 를 거부하는가(순수 판정) ──────────
    #[test]
    fn cli_only_rejects_inherited_priming_env() {
        use std::ffi::OsStr;
        // cli-only + 비어 있지 않은 env override → 충돌(true, SETUP-FAIL 유발). `--priming` co-pass 거부와 대칭.
        assert!(
            cli_only_env_override_conflicts(true, Some(OsStr::new("prompts/agent-priming.md"))),
            "cli-only 인데 상속 env override 있음 → 거부(충돌)"
        );
    }

    #[test]
    fn cli_only_ignores_empty_or_absent_priming_env() {
        use std::ffi::OsStr;
        // env 미설정(None) 또는 빈 값이면(미설정 취급) 충돌 아님 — 정상 진행.
        assert!(
            !cli_only_env_override_conflicts(true, None),
            "cli-only 인데 env 미설정 → 충돌 아님"
        );
        assert!(
            !cli_only_env_override_conflicts(true, Some(OsStr::new(""))),
            "cli-only 인데 env 빈 값(미설정 취급) → 충돌 아님"
        );
    }

    #[test]
    fn non_cli_only_never_conflicts_with_priming_env() {
        use std::ffi::OsStr;
        // 일반 모드(cli_only=false)는 env override 를 정당히 쓴다 — 값이 있어도 충돌 아님.
        assert!(
            !cli_only_env_override_conflicts(false, Some(OsStr::new("prompts/agent-priming.md"))),
            "일반 모드는 env override 정당 → 충돌 아님(cli_only=false)"
        );
    }

    // ── ADR-0099: --cli-only 엄격 성공 판정(순수) — b_sent && entrance=cli 여야 pass ────────────────
    #[test]
    fn cli_only_pass_only_when_sent_via_cli() {
        // 유일한 pass 조합: 실제 발신 + CLI 입구.
        assert!(
            cli_only_run_passed(true, "cli"),
            "b_sent=true & entrance=cli → PASS"
        );
    }

    #[test]
    fn cli_only_fail_when_nothing_sent() {
        // ★핵심(FIX 4)★: 아무것도 안 보낸 경우(b_sent=false/entrance=none)는 일반 모드의 valid-negative 와
        //   달리 cli-only 에선 FAIL(강제 false path 미실증) — pass 아님.
        assert!(
            !cli_only_run_passed(false, "none"),
            "b_sent=false/entrance=none → FAIL(pass 아님)"
        );
    }

    #[test]
    fn cli_only_fail_when_sent_via_non_cli_entrance() {
        // entrance=mcp(강제 seam 이 MCP 를 못 지움)나 그 밖의 입구는 pass 아님(이중 안전망 — 앞선 SETUP-FAIL 과
        //   별개로 순수 판정자도 cli 아닌 건 전부 실패로).
        assert!(
            !cli_only_run_passed(true, "mcp"),
            "entrance=mcp → pass 아님"
        );
        assert!(
            !cli_only_run_passed(true, "none"),
            "b_sent=true 라도 entrance=none 이면 pass 아님"
        );
    }
}
