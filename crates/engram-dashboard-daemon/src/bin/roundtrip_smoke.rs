//! roundtrip-smoke — A→B→A 왕복(reply round-trip) 실측 드라이버(검증 전용 bin).
//!
//! ## 무엇을 실측하나
//! 실 primed claude 2개(A=alice · B=bob, stream-json/Fresh)를 스폰해, priming-smoke 가 증명한 A→B
//! **수신** 위에서 두 조각만 새로 본다: ① B 가 **발신 절반**(MCP `send_message` 또는 `engram mail send` CLI)을
//! **스스로** 호출하는가 ② A 가 그 답신을 자연스럽게 수용하는가. 관측 축 둘 — 기계적 = registry
//! `DeliveryObservation`(from=B, to=A) + B 가 고른 입구, 정성적 = A 의 턴 텍스트. 최종 해석은 아래
//! stdout 마커로 오케스트레이터가 내린다.
//!
//! ## 부정 결과를 오귀속하지 않는 규율(이 하네스의 존재 이유)
//! "B 가 안 보냄"(`B_SENT=false`)은 **setup 이 온전히 성공했고 A·B 가 모두 살아 있을 때만** 유효한 실험
//! negative 다. 인프라 부재·인자 조합 오류·프로세스 사망은 전부 SETUP-SKIP / SETUP-FAIL 이라는 **다른
//! 라벨**로 끝난다. 이 셋이 섞이면 실험 데이터가 오염된다.
//!
//! ## 알려진 미확인·미통제
//! - 게이트 **판정자의 호출부 배선은 단위 테스트가 닿지 않는다** — 게이트가 실 에이전트 2개를 스폰하는
//!   `run()` 안에 있어 `if false && …` 로 꺼도 스위트는 초록이다. 커버되는 건 순수 판정자뿐이고, 이건
//!   이 파일의 **모든** 게이트에 해당하는 구조적 공백이다. 호출부를 고칠 때 테스트가 지켜준다고 가정하지 말 것.
//! - `--seed-reply-by` 는 봉투에 `reply-by` 속성을 **렌더만** 한다 — 이 하네스는 데몬의 60초 sweep 을
//!   돌리지 않는 단발 실행이라 기한 초과 타임아웃·notice 가 여기서 발화하는 일은 없다.
//! - 잘못된 기간 표기(파싱 실패)는 두 에이전트가 모두 스폰된 **뒤** 씨앗 발송 시점에야 `validate_contract`
//!   가 반려한다 — 분류는 SETUP-FAIL 이되 스폰 비용은 이미 썼다.
//!
//! ## 플래그(전부 test-only 노브 — 운영 스위치 아님)
//! - `--priming <path|C0>` — 프라이밍 파일 직접 지정(절대면 그대로, 상대면 repo 루트 기준). 미지정·`C0`
//!   = 운영 A `prompts/agent-priming.md`. 옛 C1~C3 별칭은 ADR-0099 로 제거됐다 — 지금 넘기면 특수 매핑
//!   없이 "그 이름의 파일 경로"로 해석돼 부재로 걸린다.
//! - `--model <name>` — 기본 sonnet.
//! - `--cli-only` — provision 을 비-MCP 로 강제해 CLI false path 전체를 실측한다. `--priming` co-pass 와
//!   비어 있지 않은 상속 `ENGRAM_PRIMING_FILE` 을 거부한다(둘 다 관측 대상인 auto-select 를 덮으므로).
//!   entrance=cli 를 기대한다(mcp 관측 시 SETUP-FAIL = 강제 seam 결함). ★판정이 다른 모드보다 엄격하다★
//!   — `b_sent=true AND entrance=cli` 여야 exit 0 이고, 미발신은 valid-negative 가 아니라 FAIL(강제
//!   false path 가 도는 걸 못 봤으니 목적 미달). 전용 `VERDICT [roundtrip-smoke --cli-only]:` 줄로 낸다.
//! - `--disallow-mcp` — MCP `send_message` grant 만 뺀다. ★이 노브로는 CLI 라우팅을 만들 수 없다
//!   (실측 2026-08-03 6/6 전원 MCP 정상 발신 = 조작 불성립)★ — 스폰이 MCP 가능 그대로라 그 자격증명의
//!   CLI 우편 요청은 데몬이 거절한다(ADR-0133). CLI 입구 실측은 `--cli-only` 로만 성립한다.
//!   존치·폐기는 사용자 결정 대기라 코드·플래그는 그대로 둔다.
//! - `--seed-request` (+ `--seed-reply-by <dur>`, 예 `5m`) — 씨앗 A→B 를 회신 계약으로 보낸다.
//! - `--b-task <text|@path>` — B 원과제 프롬프트 대체(`@` 접두 = 파일 참조, 상대면 repo 루트 기준).
//! - `--b-task`·`--seed-reply-by` 는 값 오용(누락 · 다음 토큰이 `--` 로 시작 · 빈 값/공백뿐)을 조용히
//!   기본값으로 흘리지 않고 **스폰 전 exit 1** 로 반려한다. `--seed-reply-by` 단독 지정(= `--seed-request`
//!   없이)도 같다. 나머지 노브는 미지정 = 오늘 동작이 곧 안전한 기본이라 관대하다.
//!
//! ## 종료 라벨(stdout+stderr 양쪽에 찍는다 — silent skip 금지)
//! - `SKIPPED` exit 0 — claude 부재/인증 실패.
//! - `SETUP-SKIP` exit 1 — 케이스가 요구하는 인프라 부재(CLI 경로인데 제어 평면 CLI 미빌드).
//! - `SETUP-FAIL` exit 1 — 인자 오류(스폰 전) · priming 파일 부재/읽기 실패 · A/B 출력 구독 실패 ·
//!   B 원과제 턴 실패 · A·B process death · 씨앗 ACK 반려.
//! - 그 밖 exit 0 — valid negative 도 exit 0 이다(유효한 실험 결과지 하네스 실패가 아니다).
//!   `--cli-only` 만 위 엄격 판정으로 예외.
//!
//! ## stdout 결과 블록 마커(오케스트레이터가 파싱한다)
//! `ROUNDTRIP CASE= B_SENT= ENTRANCE=` 배너 · `[model]` · `[priming]`(존재 검사를 통과한 **실제 in-effect
//! 경로**만) · `SEED_KIND=` · `B_TASK=` 는 **미지정이어도 항상** 찍는다 — 조용히 무시된 오타를 눈에 띄게
//! 하는 신호다(그래서 "미지정이면 오늘과 바이트 동일" 은 스폰·프라이밍·전송 로직에 대한 주장이지 stdout
//! 전문에 대한 주장이 아니다). `--seed-request` 면 다음이 붙는다:
//! - `REPLY_MATCHES_SEED` = 배달된(is_delivered) B→A 레코드 중 `in_reply_to == 씨앗 msg_id` 인 것이
//!   있는가(엄격 일치).
//! - `REPLY_IN_REPLY_TO` = 그 일치 레코드의 값을 우선해 찍고, 일치가 없을 때만 첫 배달 레코드의
//!   `in_reply_to` 로 폴백한다. 그래야 "틀린/환각 id 로 회신"(matches=false 인데 값이 있음)과
//!   "in-reply-to 를 아예 안 실음"(none)이 갈린다.
//! - `REPLY_POLL=matched|timeout|skipped-no-budget` · `REPLY_POLL_BUDGET_MS=<ms>` = 회신 폴링의 예산
//!   상태. 수치 0 = 예산 없음(스킵) **또는** 1ms 미만 잔여(절사) — 스킵 판정의 정본은 라벨이지 수치가
//!   아니다. 라벨만으로는 "굶주린 예산 끝의 timeout" 과 "정상 예산을 다 쓰고 못 찾은 negative" 가 안 갈린다.
//! `[delivery-census]` 는 축 필터 없이 캡처된 전체 배달 레코드를 도착 순서로 덤프한다.
//!
//! ## 실행(오케스트레이터가 런타임에 돌린다 — 이 파일은 빌드/컴파일만)
//! `--cli-only` 는 CLI 입구 바이너리를 **먼저** 빌드해야 한다 — `cargo run` 은 dep bin 을 안 만들고,
//! 하네스는 자기 exe 형제에서 `engram`(Win: `.exe`) 를 찾으므로 같은 profile/target 이어야 co-locate 된다.
//! ```text
//! cargo build -p engram-dashboard-daemon --features test-harness --bin engram
//! cargo run   -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- <flags>
//! ```
//! **실행 전 부모 env 의 `ENGRAM_PRIMING_FILE` 을 직접 걷어낼 것** — `--cli-only` 는 이 값이 비어 있지
//! 않으면 SETUP-FAIL 로 거부한다(operator 가 일부러 세운 값일 수 있어 조용히 지우지 않는다).
//!
//! ## 핵심 불변식
//! - **required-features = ["test-harness"]** — 운영/릴리즈 빌드는 이 bin 을 컴파일하지 않는다.
//! - **B 의 답신은 실 입구로만** — 하네스는 B 의 답신에 대해 `handle_send` 를 대신 부르지 않는다.
//!   이게 이 하네스가 새로 검증하는 것 자체다.
//! - **CLI 입구를 쓰는 실험 = `--cli-only` 한 모드뿐**(ADR-0128 채널 단일화) — MCP 가능 스폰의 CLI 우편은
//!   데몬이 자격증명으로 거절하므로(ADR-0133), CLI-지시 프라이밍을 `--priming` 으로 얹는 조합은 스폰 전에
//!   거부된다. CLI-요구 판정은 셀렉터·basename 이 아니라 **해석된 프라이밍 파일 본문**으로 한다.
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
    OutputSink, SinkError, SinkId, StatusSink, CLI_EXE_NAME,
};
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, CommandTableSlot, ManagerSlot, MessagingSlot, RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::priming::{
    mentions_mail_cli_surface, teaches_mail_cli, FilePrimingProvider, PrimingProvider,
};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::messaging_host::messaging_for_manager;
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
const TURN_WAIT_CAP: Duration = Duration::from_secs(180);
/// B 답신 대기 상한 — 초과는 에러가 아니라 negative(B did not send) 결과다.
const REPLY_WAIT_CAP: Duration = Duration::from_secs(180);

/// B 는 이 이름을 봉투에서 배워 `to=alice` 로 답신한다 — 하네스가 알려주지 않는다.
const NAME_A: &str = "alice";
const NAME_B: &str = "bob";

/// 씨앗 전에 "일하는 팀원" 맥락을 세우는 원과제 — 그 맥락이 없으면 답신이 자연 반응인지 판정할 수 없다.
const TASK_PROMPT_B: &str =
    "You are currently working on the auth module (login/session). When you're ready to start, reply in one line.";

/// ★씨앗 A→B(ADR-0092 — 자연 팀원 질문, 기계적 "툴 X 써라" 아님)★: A 가 B 에게 진행 상황을 묻는
///   평범한 협업 질문 → 답을 A 에게 돌려주는 게 자연스러운 반응이 되도록 만든다. 발신 방법(툴/CLI)은
///   본문이 아니라 **프라이밍 변형**이 가르친다(C0/기본 = 프로덕션 A `prompts/agent-priming.md` —
///   ADR-0126 결정 1 이후 send_message 만 가르친다).
const SEED_A_TO_B: &str =
    "Can you share the status of the auth module? If you're stuck anywhere on the login path, tell me what you need too.";

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run()));
}

fn skip_no_claude(reason: &str) -> i32 {
    let line =
        format!("SKIPPED [roundtrip-smoke]: {reason} — A→B→A 왕복 실측 불가(claude 부재/인증).");
    println!("{line}");
    eprintln!("{line}");
    0
}

fn setup_skip(reason: &str) -> i32 {
    let line = format!("SETUP-SKIP [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

fn setup_fail(reason: &str) -> i32 {
    let line = format!("SETUP-FAIL [roundtrip-smoke]: {reason}");
    println!("{line}");
    eprintln!("{line}");
    1
}

/// MCP 가능 스폰의 CLI 우편은 데몬이 거절한다(ADR-0133) — 그래서 CLI 발신을 가르치는 프라이밍은
///   `--cli-only` 와 함께여야만 실행 가능한 조합이 된다. 어긋나면 "가르쳤지만 쓸 수 없는 채널" 이 되어 발신
///   freeze(ADR-0099 실측 ~6/7 미발신)를 재현하고, 하네스는 그걸 정상 negative 로 오귀속한다.
/// send_exe 존재 여부는 이 판정에 **들어오지 않는다** — 바이너리를 부를 수 있어도 그 요청이 거절되기
///   때문이다(그 축은 뒤따르는 CLI 입구 부재 SETUP-SKIP 이 본다).
// ADR-0133
fn cli_priming_requires_cli_only(directs_cli: bool, cli_only: bool) -> bool {
    directs_cli && !cli_only
}

/// 역방향 — 쓸 수 있는 채널을 아무도 안 가르치는 경우. `--cli-only` 는 CLI 우편을 실제로 여는데, 그 스폰의
///   프라이밍이 CLI 발신을 안 가르치면 B 는 방법을 모른 채 조용히 아무것도 안 보내고 그 `B_SENT=false` 가
///   정상 negative 로 채점된다 — **같은 오귀속의 반대 방향**이다. 뒤쪽 `entrance != "cli"` 검사는
///   *보냈는데 입구가 틀린* 경우만 잡으므로 그 빈칸을 이 스폰-전 게이트가 메운다.
/// ★실제 발화 경로는 하나뿐 — 즉 2차 방어선이다★: 이 모드는 `--priming` co-pass 와 비어 있지 않은 상속
///   `ENGRAM_PRIMING_FILE` 을 앞서 거부하므로 이 갈래의 프라이밍은 **항상** 운영 B 파일이다. 그러니 이
///   게이트가 걸리는 경우는 그 파일이 셸 명령 언급을 잃는 개정뿐이고, 그 회귀는
///   `production_priming_files_pin_taught_channels`(priming.rs)가 1차로 잡는다. pin 이 지워지거나 override
///   정책이 느슨해지면 이쪽이 남는다.
// ADR-0128
fn cli_only_requires_cli_priming(directs_cli: bool, cli_only: bool) -> bool {
    cli_only && !directs_cli
}

fn cli_only_env_override_conflicts(cli_only: bool, env_value: Option<&std::ffi::OsStr>) -> bool {
    cli_only && matches!(env_value, Some(v) if !v.is_empty())
}

/// entrance="mcp"(강제 seam 이 MCP 를 못 지움)는 앞선 SETUP-FAIL 이 이미 잡지만, 순수 판정자 수준에서도
///   cli 아닌 건 전부 실패로 매핑해 이중 안전망을 둔다.
fn cli_only_run_passed(b_sent: bool, entrance_label: &str) -> bool {
    b_sent && entrance_label == "cli"
}

fn reply_poll_label(matched: bool, poll_ran: bool) -> &'static str {
    if matched {
        "matched"
    } else if poll_ran {
        "timeout"
    } else {
        "skipped-no-budget"
    }
}

fn seed_reply_by_without_request_is_invalid(
    seed_request: bool,
    seed_reply_by: &Option<String>,
) -> bool {
    !seed_request && seed_reply_by.is_some()
}

/// 존재 검사는 하지 않는다 — 호출자가 읽기 시도로 판정한다(`resolve_priming_path` 와 같은 분업).
fn resolve_b_task_file_path(value: &str, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel = value.strip_prefix('@')?;
    let p = PathBuf::from(rel);
    Some(if p.is_absolute() {
        p
    } else {
        repo_root.join(p)
    })
}

/// `*_error` 필드 = `parse_args` 가 오용 **사실만** 순수하게 기록한 것. 반려 여부·시점은 `run()` 이
///   정한다(실 claude 를 스폰하기 전에 SETUP-FAIL 로 fail-fast).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    priming: Option<String>,
    model: String,
    disallow_mcp: bool,
    cli_only: bool,
    seed_request: bool,
    seed_reply_by: Option<String>,
    seed_reply_by_error: Option<String>,
    b_task: Option<String>,
    b_task_error: Option<String>,
}

struct CapturingObserver {
    records: Mutex<Vec<DeliveryObservation>>,
}

impl CapturingObserver {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
    fn record_count(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    /// ★왜 baseline 절단인가★: observer 는 B 의 원과제 턴 **전에** 설치된다. B 가 그 턴에서 A 에게 메시지를
    ///   하나 흘리면 그 pre-seed 레코드가 "답신" 으로 오인돼 거짓 `B_SENT=true` 로 실험을 오염시킨다. 그래서
    ///   씨앗 주입 직전 `record_count` 를 baseline 으로 잡고 그 이후만 훑는다.
    fn find_delivery_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Option<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)?
            .iter()
            .find(|r| r.from.peer_id == from_id && r.to_id == to_id)
            .cloned()
    }

    /// 회신 **내용** 검증(REPLY_IN_REPLY_TO/REPLY_MATCHES_SEED)은 first-wins 면 안 되므로 전부를 돌려준다 —
    ///   첫 레코드가 회신과 무관한 "ack" 한 통이고 진짜 회신이 그 뒤에 오면 거짓 negative 가 난다.
    ///   b_sent/entrance 판정은 첫 도착(`find_delivery_after`)으로 충분하다.
    fn records_after(
        &self,
        baseline: usize,
        from_id: AgentId,
        to_id: AgentId,
    ) -> Vec<DeliveryObservation> {
        let recs = self.records.lock().unwrap();
        recs.get(baseline..)
            .map(|slice| {
                slice
                    .iter()
                    .filter(|r| r.from.peer_id == from_id && r.to_id == to_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn all_records(&self) -> Vec<DeliveryObservation> {
        self.records.lock().unwrap().clone()
    }
}

impl DeliveryObserver for CapturingObserver {
    fn observe(&self, obs: DeliveryObservation) {
        self.records.lock().unwrap().push(obs);
    }
}

async fn run() -> i32 {
    let args = parse_args(std::env::args().skip(1));
    // 인자 가드 3종은 priming 해석·MCP 서버 기동보다 **먼저** 돈다 — 실 claude 를 스폰하기 전에 걸러야
    //   헛된 스폰이 없다. 값 문법 오류가 의미 검사(단독 지정 등)보다 앞서는 이유는, 값이 애초에 안 먹혔다면
    //   그 의미를 논해도 무의미하기 때문이다.
    if let Some(reason) = &args.seed_reply_by_error {
        return setup_fail(reason);
    }
    if seed_reply_by_without_request_is_invalid(args.seed_request, &args.seed_reply_by) {
        return setup_fail(
            "--seed-reply-by requires --seed-request (reply_by is only meaningful as the deadline of a request contract) — add --seed-request or drop --seed-reply-by",
        );
    }
    if let Some(reason) = &args.b_task_error {
        return setup_fail(reason);
    }

    let repo_root = repo_root_from_manifest();
    let priming_selector = args.priming.clone();
    if args.cli_only && priming_selector.is_some() {
        return setup_fail(
            "--cli-only 는 --priming override 와 함께 쓸 수 없다 — 이 모드는 provision 이 자동으로 prompts/agent-priming-cli.md 를 고르는 걸 관측하는 게 목적이다(override 를 주면 그 관측이 무의미)",
        );
    }
    if cli_only_env_override_conflicts(
        args.cli_only,
        std::env::var_os("ENGRAM_PRIMING_FILE").as_deref(),
    ) {
        return setup_fail(
            "--cli-only 인데 부모 env 에 ENGRAM_PRIMING_FILE 이 설정돼 있다 — 이 override 가 provision 의 CliOnly auto-select 를 덮어써 관측을 무의미하게 만든다. 조용히 지우지 않으니(숨은 의도 파괴 방지) 실행 전에 직접 unset 하라",
        );
    }
    let priming_selector_for_resolve = if args.cli_only {
        Some("prompts/agent-priming-cli.md")
    } else {
        priming_selector.as_deref()
    };
    let resolved_priming = match resolve_priming_path(priming_selector_for_resolve, &repo_root) {
        Some(p) => p,
        None => {
            return setup_fail(&format!(
                "priming 셀렉터({priming_selector:?})를 절대경로로 못 풂 — 실험 불가"
            ));
        }
    };
    // `FilePrimingProvider` 는 존재하지 않는 override 를 조용히 버리고 UNPRIMED 로 스폰한다 — 그래서 여기서
    //   직접 확인하지 않으면 케이스 라벨이 거짓이 된다.
    if !resolved_priming.is_file() {
        return setup_fail(&format!(
            "priming 파일 없음: {} (case={:?}) — 존재하지 않는 override 는 UNPRIMED 스폰으로 이어져 케이스 라벨을 거짓으로 만든다",
            resolved_priming.display(),
            priming_selector
        ));
    }
    // ★한 번만 읽어 아래 CLI-요구 가드가 재사용한다 — 되돌리지 마라★: 이전 판본은 존재 검사와 가드에서
    //   파일을 두 번 만졌고(TOCTOU 창), 가드 쪽 `read_to_string(...).unwrap_or(false)` 가 읽기 실패(공유
    //   위반·권한·검사 후 교체·비-UTF-8)를 전부 "CLI 요구 아님" 으로 삼켜 헛된 정상 negative 를 냈다.
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
    // 여기까지는 아무것도 스폰하지 않은 시점이라 실패 반환에 정리(cleanup)할 리소스가 없다.
    let (task_prompt_b, b_task_kind): (String, String) = match &args.b_task {
        None => (TASK_PROMPT_B.to_string(), "default".to_string()),
        Some(v) => match resolve_b_task_file_path(v, &repo_root) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let kind = format!("file:{}", path.display());
                    (content, kind)
                }
                Err(e) => {
                    return setup_fail(&format!(
                        "--b-task 파일 읽기 실패: {} : {e} — B 원과제 프롬프트를 못 구했으니 진행 불가",
                        path.display()
                    ));
                }
            },
            None => {
                let kind = format!("inline({} bytes)", v.len());
                (v.clone(), kind)
            }
        },
    };
    // 아래 세 env 는 provider·manager 배선 **전에** 세워야 두 에이전트(A·B)가 같은 프라이밍 변형·같은
    //   grant 셋으로 provision 된다.
    if !args.cli_only {
        std::env::set_var("ENGRAM_PRIMING_FILE", &resolved_priming);
    }
    eprintln!(
        "[roundtrip] priming = {} (case={:?}, cli_only={})",
        resolved_priming.display(),
        priming_selector,
        args.cli_only
    );
    if args.disallow_mcp {
        std::env::set_var("ENGRAM_DISALLOW_MCP_SEND", "1");
        eprintln!("[roundtrip] --disallow-mcp → MCP send grant 제거(CLI-only 측정, ENGRAM_DISALLOW_MCP_SEND=1)");
    }
    if args.cli_only {
        std::env::set_var("ENGRAM_FORCE_CLI_ONLY_SEND", "1");
        eprintln!("[roundtrip] --cli-only → provision 을 비-MCP 로 강제(false path 전체, ENGRAM_FORCE_CLI_ONLY_SEND=1); entrance=cli 기대");
    }

    let registry = Arc::new(ControlRegistry::new());
    let observer = Arc::new(CapturingObserver::new());
    registry.set_delivery_observer(observer.clone());

    let slot = Arc::new(ManagerSlot::new());
    // C1: MessagingService 늦은 주입 슬롯 — send/flush 담당(manager 조립 후 채운다).
    let messaging_slot = Arc::new(MessagingSlot::new());
    let handle = match start_mcp_server(
        registry.clone(),
        slot.clone(),
        messaging_slot.clone(),
        // 스모크/하네스에는 붙을 클라이언트가 없다 — 명부 통지 팬아웃은 비운다(ADR-0132).
        Arc::new(RosterBroadcastSlot::new()),
        Arc::new(CommandTableSlot::new()),
    )
    .await
    {
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

    let send_exe = sibling_send_exe();
    match &send_exe {
        Some(p) => eprintln!("[roundtrip] {CLI_EXE_NAME} = {}", p.display()),
        None => {
            eprintln!("[roundtrip] {CLI_EXE_NAME} 형제 바이너리 없음 — CLI 입구 비활성(MCP 만).")
        }
    }
    // 금지 방향은 넓은 신호(의심스러우면 막는다), 요구 방향은 강한 신호(명령을 실제로 가르쳐야 한다).
    //   두 게이트가 같은 신호를 쓰면 한쪽이 반드시 틀린다 — 근거는 그 두 함수의 주석.
    let directs_cli = teaches_mail_cli(&priming_content);
    let mentions_cli_surface = mentions_mail_cli_surface(&priming_content);
    if cli_priming_requires_cli_only(mentions_cli_surface, args.cli_only) {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_fail(&format!(
            "CLI-teaching priming on an MCP-capable spawn (case={:?}): MCP 가능 스폰의 CLI 우편 요청은 데몬이 자격증명으로 거절하므로(ADR-0133) B 가 배운 명령이 통하지 않는다 — 가르쳤지만 쓸 수 없는 채널 = 발신 freeze(ADR-0099 실측 ~6/7 미발신)를 정상 negative 로 오귀속하게 된다. CLI 입구를 실측하려면 `--cli-only` 로 provision 을 비-MCP 로 정렬하라(그 모드가 CliOnly 프라이밍을 자동 선택하므로 `--priming` 은 함께 주지 않는다). ※이 금지 방향의 판정은 **넓게** 본다(`mentions_mail_cli_surface`) — 우편 CLI 호출 형태(`engram mail …`/`$ENGRAM_CLI_EXE mail …`)·env 변수 이름 자체·CLI 전용 플래그(`--to`·`--body`·`--body-stdin`·`--request`·`--reply-by`·`--reply-to`) 중 **하나라도 등장하면** 걸린다. 모든 arm 은 같은 정규화(소문자 · 공백/줄바꿈 축약 · 마크다운 강조/이스케이프 기호 제거 · HTML 주석 제거 · 제로폭 문자 제거)를 거친 뒤 **낱말 경계**로 비교하므로, 대소문자·공백/줄바꿈·마크다운 강조·HTML 주석·제로폭 문자 정도의 흐트러짐은 접히고 `engram mailbox`·`--token` 같은 평범한 낱말은 걸리지 않는다. **셸 문법 일반(변수 재바인딩·별칭)과 문장 부정은 판정하지 않는다** — 그 한계는 `teaches_mail_cli` 주석이 정본이다. 부정문(\"그 명령을 쓰지 마라\")은 변수 **맨 언급**으로 여전히 이 금지 방향에 걸린다(의도된 fail-closed: 오귀속보다 거부가 안전하다 — 다만 그건 '가르쳤다' 는 판정은 아니다). MCP 전용 파일로 돌리려면 그 언급 자체를 빼라",
            priming_selector
        ));
    }
    if cli_only_requires_cli_priming(directs_cli, args.cli_only) {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_fail(
            "--cli-only 인데 해석된 프라이밍이 CLI 발신을 가르치지 않는다 — 이 모드는 provision 을 비-MCP 로 정렬해 CLI 우편을 열지만, 가르치지 않으면 B 는 발신 방법을 모른 채 아무것도 보내지 않고 그 B_SENT=false 가 정상 negative 로 오귀속된다(가르치는 채널 == 쓸 수 있는 채널). prompts/agent-priming-cli.md 가 그 명령을 가르치는지 확인하고, 상속된 ENGRAM_PRIMING_FILE override 가 MCP 전용 파일을 가리키고 있지 않은지 보라",
        );
    }
    // 위 두 게이트를 통과했으면 `directs_cli ⟺ cli_only` 가 성립한다 — 그 상태에서 send_exe 가 없으면
    //   B 는 물리적으로 못 보내므로 B_SENT=false 는 실험 결과가 아니라 인프라 부재다.
    if directs_cli && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(&format!(
            "control CLI not built, inlet unavailable (case={:?} requires the CLI send path). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` 로 형제 위치에 빌드하라",
            priming_selector
        ));
    }
    // ★이 스킵이 지키는 것은 "CLI grant 가 있다" 가 아니라 하네스의 SETUP 라벨 일관성뿐이다★ — 이 모드의
    //   발신 grant 는 바이너리 유무와 무관하게 0 이다(아래 문구 참조). 노브 존치·폐기가 사용자 결정 대기라
    //   동작을 그대로 둔 것이다.
    if args.disallow_mcp && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--disallow-mcp: 제어 평면 CLI 형제 바이너리가 없어 SETUP-SKIP. ※주의(ADR-0133) — 이 모드는 스폰을 MCP 가능 그대로 두므로 바이너리를 빌드하면 명령은 실행되지만 **그 요청을 데몬이 거절한다**(MCP 가능 자격증명의 CLI 우편 입구는 닫혀 있다). 게다가 auto 권한 모드에선 grant 가 NO-OP 이라 에이전트는 그대로 MCP 로 보낸다(실측 6/6). 즉 빌드는 이 스킵을 넘기기 위한 절차일 뿐 CLI 라우팅을 만들지 못한다 — CLI 입구를 실측하려면 `--cli-only` 를 쓰라. 스킵을 넘기려면: `cargo build -p engram-dashboard-daemon --features test-harness --bin engram`",
        );
    }
    // ★오늘은 도달 불가 — 그래도 남긴다★: 위 `directs_cli ⟺ cli_only` 때문에 바로 위 스킵이 항상 먼저
    //   걸린다. 지우지 않는 이유는 그 도달 불가가 **다른 게이트의 성질에 의존**하기 때문이다 —
    //   `teaches_mail_cli` 가 좁아지거나 역방향 게이트가 완화되면 이 조합이 되살아나고, 그때 이게
    //   없으면 provision 의 fail-closed(Err)를 스폰 뒤에 SETUP-FAIL 로 늦게 만난다(진단이 나빠진다).
    //   도달 불가라 테스트로 고정할 수 없다 — 순서를 바꿔 "되살리는" 리팩터를 하지 말 것.
    if args.cli_only && send_exe.is_none() {
        handle.shutdown().await;
        let dirs = [&data_dir, &ws_a, &ws_b];
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
        return setup_skip(
            "--cli-only requires the CLI inlet but it is not built — forced non-MCP spawn has no MCP inlet, and no CLI grant means agents have no send path (provision would fail-closed). 먼저 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` 로 형제 위치에 빌드하라",
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
    // C1: MessagingService 조립(발송 3분기·flush) — manager 를 DeliveryPort 로 감싼다. 이 하네스는
    //   flush sink 를 배선하지 않으므로(NoopStatus) 파킹 시나리오는 handle_single_send 직접 경로만 탄다.
    //   씨앗 A→B 는 산 수신자라 delivered 경로.
    let messaging = Arc::new(messaging_for_manager(manager.clone(), registry.clone()));
    messaging_slot.set(messaging.clone());

    // ── A·B 스폰(둘 다 실 primed claude, stream-json, Fresh) ─────────────────────────
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

    let obs_a = Arc::new(TurnObserver::new());
    let obs_b = Arc::new(TurnObserver::new());
    let sink_a = manager.subscribe(agent_a.id, obs_a.clone()).ok();
    let sink_b = manager.subscribe(agent_b.id, obs_b.clone()).ok();

    macro_rules! fail_setup {
        ($reason:expr) => {{
            print_delivery_census(&observer);
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

    if sink_a.is_none() {
        fail_setup!("A 출력 구독 실패(sink_a=None) — A 턴 관측 불가, 정성 결과 무의미(setup 실패)");
    }
    if sink_b.is_none() {
        fail_setup!("B 출력 구독 실패(sink_b=None) — B 턴 관측 불가, setup 판정 불가(setup 실패)");
    }

    // ── 1) B 원과제 턴(일하는 팀원 맥락) ────────────────────────────────────────────
    // ★warn 후 계속하던 옛 형태로 되돌리지 마라★: 팀원 맥락이 서지 않은 채 `B_SENT=false` 를 정상
    //   negative 로 보고해 setup 실패를 실험 결과로 오인한다.
    if !send_and_wait(&manager, agent_b.id, &obs_b, &task_prompt_b) {
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
    obs_a.begin_turn();
    let baseline_a = obs_a.done_snapshot();
    let reply_baseline = observer.record_count();
    obs_b.begin_turn();
    let baseline_b = obs_b.done_snapshot();

    let seed_contract = if args.seed_request {
        SendContract {
            request: true,
            reply_by: args.seed_reply_by.clone(),
            reply_to: None,
        }
    } else {
        SendContract::default()
    };
    let seed = ControlCommand {
        from: from_a,
        to: vec![NAME_B.to_string()],
        body: SEED_A_TO_B.to_string(),
        contract: seed_contract,
    };
    let ack = handle_send(&manager, &registry, &messaging, Entrance::Cli, seed);
    eprintln!("[roundtrip] seed A→B ACK = {}", ack.to_json());
    // B 는 산 수신자라 파킹이 아니라 접수(delivered)되어야 한다 — 그래서 `is_accepted` 로 반려만 거른다.
    if !ack.is_accepted() {
        fail_setup!(&format!(
            "씨앗 A→B ACK 가 접수 실패(반려): {}",
            ack.to_json()
        ));
    }
    // ACK JSON 의 `id` 가 곧 이 씨앗의 논리 msg_id 다(handle_send 성공 응답 shape, spec §6) — 회신 일치
    //   판정(REPLY_MATCHES_SEED)의 기준값.
    let seed_msg_id: Option<String> = ack
        .to_json()
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // ★ACK shape-drift 가드 — 오늘은 발화 불가★: 현재 `ControlResult::Ok { id: String, .. }`(ingress.rs)
    //   상 접수된 ACK 는 `id` 를 항상 싣는다. 그래도 두는 이유는, 그 shape 이 바뀌어 조용히 None 이 나오면
    //   REPLY_POLL=skipped-no-budget(예산 고갈)으로 **오분류**되기 때문이다 — 실제 원인은 기준값 부재라
    //   애초에 일치를 판정할 수 없는 것인데 같은 라벨 뒤에 숨는다.
    if args.seed_request && seed_msg_id.is_none() {
        fail_setup!(&format!(
            "seed ACK missing id — request judgment impossible (ACK={})",
            ack.to_json()
        ));
    }

    // ── 3) B 의 답신을 **B 자신의 발신 경로**로 대기(하네스는 handle_send 를 부르지 않는다) ──────
    // ★이 시각부터 재는 REPLY_WAIT_CAP 은 아래 `wait_for_reply` 만의 예산이 아니다(사용자 결정 option b)★:
    //   그 뒤 이어지는 A 턴 대기(`wait_turn_end`, 최대 TURN_WAIT_CAP)까지 **같은 벽시계를 함께 태운다** —
    //   새 타임아웃 상수를 만들지 않고 창 하나를 공유시킨 트레이드오프다. A 턴이 길면 뒤의
    //   `wait_for_matching_reply` 잔여 예산이 0/음수가 돼 폴링이 아예 안 돈다.
    let reply_wait_started = Instant::now();
    let reply_obs = wait_for_reply(
        &observer,
        reply_baseline,
        agent_b.id,
        agent_a.id,
        REPLY_WAIT_CAP,
    );
    let b_sent = reply_obs.is_some();
    // ★A 생존도 함께 본다 — B-only 게이트로 되돌리지 마라★: A 가 죽으면 B 의 답신이 도달할 대상이 없어
    //   관측이 안 뜨는데 B 는 살아 있어, B 만 보는 게이트는 그걸 정상 negative 로 오분류한다.
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
    // ★`b_sent` 조건이 붙은 이유★: entrance=none(B 미발신)은 여기서 안 잡고 끝의 엄격 VERDICT 가 FAIL 로
    //   처리한다. 여기 SETUP-FAIL 은 "seam 배관 결함"(mcp 가 새어나옴) 전용이고, "강제 false path 미실증"은
    //   결과 판정이라 — 서로 다른 실패 원인을 한 라벨에 섞지 않는다.
    if args.cli_only && b_sent && entrance_label != "cli" {
        fail_setup!(&format!(
            "--cli-only 인데 B 가 entrance={entrance_label} 로 발신 — 강제 seam 이 MCP 입구를 제거 못 함(배관 결함, 정상 negative 아님)"
        ));
    }

    // ── 4) A 가 B 답신을 처리하며 낸 텍스트 대기(정성 관측) ───────────────────────────
    let a_responded = if b_sent {
        obs_a.wait_turn_end(baseline_a, TURN_WAIT_CAP)
    } else {
        // B 가 안 보냈으면 A 턴이 돌 이유가 없다 — 이미 REPLY_WAIT_CAP 동안 아무것도 없었으므로 대기 없이 본다.
        obs_a.done_snapshot() > baseline_a
    };
    let a_response = obs_a.response_text();

    // ★진짜 계약 회신을 여기서 마저 기다린다★: `wait_for_reply` 는 baseline 이후 **첫** B→A 레코드에서
    //   멈추는데 그게 회신과 무관한 "ack" 한 통일 수 있어, 바로 스캔하면 거짓 negative 가 난다.
    //   seed-request 일 때만 한다 — plain 씨앗은 `in_reply_to` 자체가 없어 폴링할 대상이 없고, 기본 경로를
    //   늦추지 않아야 한다.
    let mut reply_poll_ran = false;
    let mut reply_poll_budget_ms: u64 = 0;
    if args.seed_request {
        if let Some(sid) = seed_msg_id.as_deref() {
            let remaining = REPLY_WAIT_CAP.saturating_sub(reply_wait_started.elapsed());
            reply_poll_budget_ms = remaining.as_millis() as u64;
            if !remaining.is_zero() {
                reply_poll_ran = true;
                wait_for_matching_reply(
                    &observer,
                    reply_baseline,
                    agent_b.id,
                    agent_a.id,
                    sid,
                    remaining,
                );
            }
        }
    }

    // ── 5) 구조화 stdout 마커(오케스트레이터 판정용) ────────────────────────────────
    // cli-only 모드엔 셀렉터가 없으므로(override 금지) 전용 CASE 라벨을 단다.
    let case_label = if args.cli_only {
        "CLI-ONLY(forced non-MCP)"
    } else {
        priming_selector.as_deref().unwrap_or("C0")
    };
    println!("\n===== ROUNDTRIP CASE={case_label} B_SENT={b_sent} ENTRANCE={entrance_label} =====");
    println!("[model] {}", args.model);
    println!("[priming] {}", resolved_priming.display());
    println!("[seed A->B body] {SEED_A_TO_B}");
    let seed_kind = if args.seed_request {
        "request"
    } else {
        "plain"
    };
    println!("SEED_KIND={seed_kind}");
    println!("B_TASK={b_task_kind}");
    if args.seed_request {
        println!(
            "SEED_REPLY_BY={}",
            args.seed_reply_by.as_deref().unwrap_or("none")
        );
        // ★배달된(is_delivered) 레코드만 증거로 인정한다★ — write 가 실패한 레코드는 실제로 도달하지
        //   않았으므로, 그 `in_reply_to` 가 우연히 seed id 와 같아도 "B 가 성공적으로 회신했다" 의 증거가
        //   아니다. 그런 레코드만 남으면 정직하게 none/false 로 낸다.
        let after_seed = observer.records_after(reply_baseline, agent_b.id, agent_a.id);
        let matched = after_seed.iter().find(|r| {
            r.is_delivered()
                && seed_msg_id
                    .as_deref()
                    .is_some_and(|s| r.in_reply_to.as_deref() == Some(s))
        });
        let reply_matches_seed = matched.is_some();
        let first_reply_with_id = after_seed
            .iter()
            .find(|r| r.is_delivered() && r.in_reply_to.is_some());
        // ★무조건 first-wins 로 되돌리지 마라★: 그러면 B 가 틀린 id 로 먼저 답하고 맞는 id 로 나중에 답할 때
        //   `REPLY_IN_REPLY_TO=<wrong-id>` 인데 `REPLY_MATCHES_SEED=true` 인 자기모순 쌍이 난다(두 마커가
        //   서로 다른 레코드를 가리킴). 매치가 있으면 그 레코드를 우선해 두 마커가 같은 레코드에서 파생되게 한다.
        let reply_in_reply_to: Option<String> = match matched {
            Some(m) => m.in_reply_to.clone(),
            None => first_reply_with_id.and_then(|r| r.in_reply_to.clone()),
        };
        println!(
            "REPLY_IN_REPLY_TO={}",
            reply_in_reply_to.as_deref().unwrap_or("none")
        );
        println!("REPLY_MATCHES_SEED={reply_matches_seed}");
        let reply_poll = reply_poll_label(reply_matches_seed, reply_poll_ran);
        println!("REPLY_POLL={reply_poll}");
        println!("REPLY_POLL_BUDGET_MS={reply_poll_budget_ms}");
    }
    println!("[B sent reply to A] {b_sent}");
    println!("[B chosen entrance] {entrance_label}");
    if let Some(o) = &reply_obs {
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
    print_delivery_census(&observer);
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
    0
}

/// `iter` 로 받아 std::env 의존을 뺀다 — 순수라 단위테스트가 닿는다.
/// 다음 토큰이 또 플래그(`--` 로 시작)면 값으로 삼키지 않는다(FIX round-2 #7) — 삼키면 `--priming --model
///   opus` 에서 `--model` 이 priming 값으로 먹혀 model 이 조용히 기본값에 남는다.
fn parse_args(iter: impl Iterator<Item = String>) -> Args {
    let mut priming = None;
    let mut model = "sonnet".to_string();
    let mut disallow_mcp = false;
    let mut cli_only = false;
    let mut seed_request = false;
    let mut seed_reply_by = None;
    let mut seed_reply_by_error: Option<String> = None;
    let mut b_task = None;
    let mut b_task_error: Option<String> = None;
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
            "--seed-request" => seed_request = true,
            "--seed-reply-by" => match it.peek() {
                None => {
                    seed_reply_by_error = Some(
                        "--seed-reply-by requires a value (duration text like '5m') but none was given."
                            .to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    seed_reply_by_error = Some(format!(
                        "--seed-reply-by requires a value (duration text like '5m'); '{next}' looks like another flag, not a duration value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        seed_reply_by_error = Some(
                            "--seed-reply-by value is empty or whitespace-only; provide a duration like '5m'."
                                .to_string(),
                        );
                    } else {
                        seed_reply_by = Some(v);
                    }
                }
            },
            "--b-task" => match it.peek() {
                None => {
                    b_task_error = Some(
                        "--b-task requires a value (text or @path) but none was given.".to_string(),
                    );
                }
                Some(next) if next.starts_with("--") => {
                    b_task_error = Some(format!(
                        "--b-task requires a value (text or @path); '{next}' looks like another flag, not a task value."
                    ));
                }
                Some(_) => {
                    let v = it.next().expect("peeked Some");
                    if v.trim().is_empty() {
                        b_task_error = Some(
                            "--b-task value is empty or whitespace-only; provide task text or an @path reference."
                                .to_string(),
                        );
                    } else {
                        b_task = Some(v);
                    }
                }
            },
            _ => {}
        }
    }
    Args {
        priming,
        model,
        disallow_mcp,
        cli_only,
        seed_request,
        seed_reply_by,
        seed_reply_by_error,
        b_task,
        b_task_error,
    }
}

fn take_flag_value<I: Iterator<Item = String>>(it: &mut std::iter::Peekable<I>) -> Option<String> {
    match it.peek() {
        Some(next) if next.starts_with("--") => None,
        Some(_) => it.next(),
        None => None,
    }
}

/// 반환은 항상 절대경로 — 존재 검사는 하지 않는다(`FilePrimingProvider` 가 최종 존재·CLI-안전 검사).
fn resolve_priming_path(selector: Option<&str>, repo_root: &std::path::Path) -> Option<PathBuf> {
    let rel: &str = match selector {
        None | Some("C0") | Some("c0") => "prompts/agent-priming.md",
        Some(path) => {
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

fn entrance_str(e: Entrance) -> &'static str {
    match e {
        Entrance::Mcp => "mcp",
        Entrance::Cli => "cli",
        Entrance::Daemon => "daemon",
    }
}

/// 전체 UUID 는 census 한 줄을 과하게 늘려 눈으로 훑기 어렵게 만든다. `.simple()` 은 하이픈 없는 32자 hex
///   문자열이라 앞 8자 슬라이스가 전부 ASCII hex임이 보장된다(byte-slice 안전).
fn short_peer_id(id: uuid::Uuid) -> String {
    id.simple().to_string()[..8].to_string()
}

/// ★왜 축 필터 없이 전부 찍나★: [B->A delivery] 는 회신 축(B→A) 한 레코드만 찍는다. 그룹(@all) 방송·B
///   원과제 턴 중 발송처럼 그 축 밖의 배달은 stdout 어디에도 안 잡혀, 인수 실측이 참가자 자기보고에
///   의존하게 되는 관측 공백이 있었다(2026-07-28 인수 ③ 실측에서 실발생).
/// ★delivered/err 축은 생략 불가(F1)★: 하네스 규율은 "배달된(is_delivered) 레코드만 증거"다 — 상태 축
///   없이 찍으면 write 실패 레코드가 배달 증거처럼 읽혀, census 가 없애려던 자기보고 의존을 자기가
///   재생산한다(false positive 제조기).
fn print_delivery_census(observer: &CapturingObserver) {
    for (idx, obs) in observer.all_records().iter().enumerate() {
        println!(
            "[delivery-census] #{idx} from={} to={}({}) entrance={} msg_id={} in_reply_to={} bytes={} delivered={} err={}",
            short_peer_id(obs.from.peer_id),
            short_peer_id(obs.to_id),
            obs.to_name,
            entrance_str(obs.entrance),
            obs.msg_id,
            obs.in_reply_to.as_deref().unwrap_or("-"),
            obs.bytes_requested,
            obs.is_delivered(),
            obs.error.as_deref().unwrap_or("-")
        );
    }
}

/// 이 실험 하네스는 항상 `cargo run` 으로 도는 컴파일타임 소스 트리 안이라 `CARGO_MANIFEST_DIR` 이 신뢰
///   가능하다(빌드된 bin 을 다른 곳으로 옮겨 실행하는 경로는 없다) — 운영 데몬의 exe-walk-up 과 다른 이유.
fn repo_root_from_manifest() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // .../crates/engram-dashboard-daemon
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

fn sibling_send_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        format!("{CLI_EXE_NAME}.exe")
    } else {
        CLI_EXE_NAME.to_string()
    };
    let cand = dir.join(name);
    cand.is_file().then_some(cand)
}

fn is_agent_alive(manager: &Arc<AgentManager>, id: AgentId) -> bool {
    manager
        .list_agents()
        .iter()
        .find(|a| a.id == id)
        .map(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Exiting))
        .unwrap_or(false)
}

fn spawn_named(
    manager: &Arc<AgentManager>,
    name: &str,
    model: &str,
    workspace: &std::path::Path,
) -> Option<AgentInfo> {
    // 두 에이전트는 같은 workspace cwd 를 공유해 basename 이 동일 → cwd 파생 이름이면 alice/bob 이 충돌·
    //   오라우팅(bob 로 답신)한다. 그래서 이름을 display_name 에 심어 **결정적으로** 구분한다
    //   (ADR-0101 WYSIWYA).
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

/// 폴링인 이유: relay 는 B 의 실 claude 판단에 달려 비결정적 지연을 가진다(cv 신호원이 없어 짧은 sleep
///   폴링이 단순·충분).
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

fn wait_for_matching_reply(
    observer: &Arc<CapturingObserver>,
    baseline: usize,
    from_b: AgentId,
    to_a: AgentId,
    seed_msg_id: &str,
    cap: Duration,
) -> Option<DeliveryObservation> {
    let deadline = Instant::now() + cap;
    loop {
        if let Some(rec) = observer
            .records_after(baseline, from_b, to_a)
            .into_iter()
            .find(|r| r.is_delivered() && r.in_reply_to.as_deref() == Some(seed_msg_id))
        {
            return Some(rec);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

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

/// ★lost-wakeup 방지(FIX round-2 #3)★: `done_count` 를 `AtomicU64` 로 두고 mutex 밖에서 증가·notify 하면,
///   waiter 의 [원자 predicate 체크] 와 [`wait_timeout` 등록] 사이에 낀 완료 신호는 wakeup 을 잃고(cv 는
///   등록 전 notify 를 기억하지 않는다) 이미 끝난 턴을 상한까지 헛대기해 **거짓 타임아웃**(A 무응답 /
///   B task 타임아웃)을 낸다. 그래서 predicate 상태(`done_count`)를 **cv 가 쓰는 바로 그 mutex 안**에
///   두고, 발신·대기 모두 그 락을 잡은 채 갱신·재확인한다.
struct TurnState {
    text: String,
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
        assert!(
            !a.seed_request,
            "기본은 씨앗 계약 없음(plain 통보, 오늘 동작)"
        );
        assert_eq!(a.seed_reply_by, None);
        assert_eq!(
            a.seed_reply_by_error, None,
            "기본은 --seed-reply-by 오용 없음"
        );
        assert_eq!(a.b_task, None, "기본은 B 원과제 override 없음(오늘 동작)");
        assert_eq!(a.b_task_error, None, "기본은 --b-task 오용 없음");
    }

    #[test]
    fn parse_args_cli_only_flag_is_boolean() {
        let a = parse_args(s(&["--cli-only", "--model", "opus"]));
        assert!(a.cli_only, "--cli-only 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--cli-only 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_cli_only_absent_is_false() {
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
        assert!(!a.cli_only);
    }

    #[test]
    fn parse_args_disallow_mcp_flag_is_boolean() {
        let a = parse_args(s(&["--disallow-mcp", "--model", "opus"]));
        assert!(a.disallow_mcp, "--disallow-mcp 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--disallow-mcp 뒤 --model 은 정상 파싱");
        assert_eq!(a.priming, None);
    }

    #[test]
    fn parse_args_disallow_mcp_absent_is_false() {
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
        let a = parse_args(s(&["--priming"]));
        assert_eq!(a.priming, None);
        assert_eq!(a.model, "sonnet");
    }

    #[test]
    fn parse_args_flag_does_not_consume_next_flag_as_value() {
        let a = parse_args(s(&["--priming", "--model", "opus"]));
        assert_eq!(
            a.priming, None,
            "다음 토큰이 플래그면 priming 값으로 삼키지 않는다"
        );
        assert_eq!(a.model, "opus", "--model 은 정상 파싱돼야");
    }

    #[test]
    fn parse_args_trailing_flag_flags_both_ignored_cleanly() {
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
        let got2 = resolve_priming_path(Some("C0"), &root).expect("C0 경로");
        assert_eq!(got, got2);
    }

    // ADR-0099: 옛 C1~C3 실험 별칭 매핑 테스트는 별칭 제거와 함께 삭제됐다 — 아래 명시 경로 override
    //   테스트가 지금 동작을 커버한다.

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

    #[test]
    fn teaches_mail_cli_true_for_cli_command_mention() {
        let text =
            "To reply, run in your shell: `$ENGRAM_CLI_EXE mail send --to alice --body ...`\n\
                    i.e. run the engram mail send command with the recipient name.";
        assert!(teaches_mail_cli(text), "우편 CLI 명령 언급 → CLI 지시");
    }

    /// ★맨 언급은 교육이 아니다(호출 형태여야 한다)★ — 변수를 "쓰지 마라" 고 적은 문장이 교육으로
    ///   세지면 요구 게이트가 거짓으로 만족된다. 규칙 정본·전수 케이스는 `control::priming` 쪽 테스트.
    #[test]
    fn env_var_mention_is_surface_evidence_and_invocation_is_teaching() {
        let mention = "Invoke the binary referenced by ENGRAM_CLI_EXE to deliver your message.";
        assert!(!teaches_mail_cli(mention), "맨 언급은 교육이 아니다");
        assert!(
            mentions_mail_cli_surface(mention),
            "그래도 CLI 표면 흔적이라 금지 방향에선 걸린다"
        );
        assert!(
            teaches_mail_cli("run `$ENGRAM_CLI_EXE mail send --to alice`"),
            "변수 호출 형태는 교육이다"
        );
    }

    #[test]
    fn teaches_mail_cli_false_for_mcp_only() {
        let text = "To reply, call the MCP tool `send_message` with the recipient and body.";
        assert!(
            !teaches_mail_cli(text),
            "MCP send_message 만 → CLI 지시 아님"
        );
    }

    #[test]
    fn teaches_mail_cli_false_for_empty() {
        assert!(!teaches_mail_cli(""), "빈 본문 → CLI 지시 아님");
    }

    #[test]
    fn teaches_mail_cli_case_insensitive() {
        assert!(
            teaches_mail_cli("Reply via ENGRAM MAIL SEND right away."),
            "대문자 표기 → CLI 지시"
        );
        assert!(
            teaches_mail_cli("Use Engram Mail Send to deliver."),
            "혼합 표기 → CLI 지시"
        );
        assert!(
            teaches_mail_cli("Invoke $Engram_Cli_Exe Mail Pending to check."),
            "혼합 표기 변수 호출 → CLI 지시"
        );
    }

    #[test]
    fn teaches_mail_cli_negation_is_intentionally_true() {
        assert!(
            teaches_mail_cli("Do NOT use engram mail send; use MCP instead."),
            "부정문도 호출 형태의 인접이 성립하면 true — 문장 부정 판정은 의도적으로 안 한다(알려진 한계)"
        );
    }

    // ── ADR-0133: 교육 축 정합(가르치는 우편 채널 == 그 스폰이 쓸 수 있는 우편 채널) ──────────────
    #[test]
    fn cli_priming_without_cli_only_is_rejected() {
        assert!(
            cli_priming_requires_cli_only(true, false),
            "CLI-지시 프라이밍 + --cli-only 없음 → 거부"
        );
    }

    #[test]
    fn cli_only_without_cli_priming_is_rejected() {
        assert!(
            cli_only_requires_cli_priming(false, true),
            "--cli-only + CLI 안 가르치는 프라이밍 → 거부"
        );
    }

    #[test]
    fn wiring_axis_accepts_only_the_two_aligned_combinations() {
        // ★OR 이 아니라 판정자별 셀을 고정하는 이유★: OR 만 보면 두 판정자가 같은 행을 함께 거부하도록
        //   넓어져도 초록이다(실측: 정방향을 `directs_cli != cli_only` 로 넓히면 OR 단언만으론 안 잡힌다).
        //   두 게이트는 operator 에게 **서로 다른 처방**을 안내하므로(`--cli-only` 를 붙여라 / 프라이밍이
        //   CLI 를 가르치는지 보라) 어느 쪽이 거부하는지가 실제 계약이다.
        for (directs_cli, cli_only, expect_forward, expect_converse) in [
            (true, true, false, false),
            (false, false, false, false),
            (true, false, true, false),
            (false, true, false, true),
        ] {
            let forward = cli_priming_requires_cli_only(directs_cli, cli_only);
            let converse = cli_only_requires_cli_priming(directs_cli, cli_only);
            assert_eq!(
                forward, expect_forward,
                "정방향(cli_priming_requires_cli_only) directs_cli={directs_cli} cli_only={cli_only}"
            );
            assert_eq!(
                converse, expect_converse,
                "역방향(cli_only_requires_cli_priming) directs_cli={directs_cli} cli_only={cli_only}"
            );
            // 호출부 결정(두 게이트가 OR 로 붙는다)도 함께 고정 — 라우팅과 별개 축이라 지우지 않는다.
            assert_eq!(
                forward || converse,
                expect_forward || expect_converse,
                "호출부 거부 여부 directs_cli={directs_cli} cli_only={cli_only}"
            );
        }
    }

    // ── ADR-0099: --cli-only 가 상속된 ENGRAM_PRIMING_FILE override 를 거부하는가(순수 판정) ──────────
    #[test]
    fn cli_only_rejects_inherited_priming_env() {
        use std::ffi::OsStr;
        assert!(
            cli_only_env_override_conflicts(true, Some(OsStr::new("prompts/agent-priming.md"))),
            "cli-only 인데 상속 env override 있음 → 거부(충돌)"
        );
    }

    #[test]
    fn cli_only_ignores_empty_or_absent_priming_env() {
        use std::ffi::OsStr;
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
        assert!(
            !cli_only_env_override_conflicts(false, Some(OsStr::new("prompts/agent-priming.md"))),
            "일반 모드는 env override 정당 → 충돌 아님(cli_only=false)"
        );
    }

    // ── ADR-0099: --cli-only 엄격 성공 판정(순수) ───────────────────────────────────────────────
    #[test]
    fn cli_only_pass_only_when_sent_via_cli() {
        assert!(
            cli_only_run_passed(true, "cli"),
            "b_sent=true & entrance=cli → PASS"
        );
    }

    #[test]
    fn cli_only_fail_when_nothing_sent() {
        assert!(
            !cli_only_run_passed(false, "none"),
            "b_sent=false/entrance=none → FAIL(pass 아님)"
        );
    }

    #[test]
    fn cli_only_fail_when_sent_via_non_cli_entrance() {
        assert!(
            !cli_only_run_passed(true, "mcp"),
            "entrance=mcp → pass 아님"
        );
        assert!(
            !cli_only_run_passed(true, "none"),
            "b_sent=true 라도 entrance=none 이면 pass 아님"
        );
    }

    // ── REPLY_POLL 라벨 판정(순수) ──────────────────────────────────────────────────────────────
    #[test]
    fn reply_poll_label_matched_wins_regardless_of_poll_ran() {
        assert_eq!(reply_poll_label(true, true), "matched");
        assert_eq!(
            reply_poll_label(true, false),
            "matched",
            "matched 는 poll_ran 과 무관하게 최우선(skipped-no-budget 보다 이긴다)"
        );
    }

    #[test]
    fn reply_poll_label_timeout_when_ran_but_not_matched() {
        assert_eq!(reply_poll_label(false, true), "timeout");
    }

    #[test]
    fn reply_poll_label_skipped_no_budget_when_not_ran_and_not_matched() {
        assert_eq!(reply_poll_label(false, false), "skipped-no-budget");
    }

    // ── --seed-request/--seed-reply-by(순수 파싱) ─────────────────────────────────────────────────
    #[test]
    fn parse_args_seed_request_flag_is_boolean() {
        let a = parse_args(s(&["--seed-request", "--model", "opus"]));
        assert!(a.seed_request, "--seed-request 존재 → 켜짐");
        assert_eq!(a.model, "opus", "--seed-request 뒤 --model 은 정상 파싱");
        assert_eq!(a.seed_reply_by, None);
    }

    #[test]
    fn parse_args_seed_request_absent_is_false() {
        let a = parse_args(s(&["--priming", "C0", "--model", "haiku"]));
        assert!(!a.seed_request);
    }

    #[test]
    fn parse_args_seed_reply_by_value() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert!(a.seed_request);
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
    }

    #[test]
    fn parse_args_seed_reply_by_does_not_consume_next_flag_as_value() {
        let a = parse_args(s(&["--seed-reply-by", "--model", "opus"]));
        assert_eq!(
            a.seed_reply_by, None,
            "다음 토큰이 플래그면 seed_reply_by 값으로 삼키지 않는다"
        );
        assert!(
            a.seed_reply_by_error.is_some(),
            "다음 토큰이 플래그처럼 보이면 오용 에러를 기록해야(F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model 은 정상 파싱돼야(에러 기록이 뒤 파싱을 막지 않는다)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by"]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "값 없이 끝나는 --seed-reply-by 는 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_empty_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", ""]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "빈 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "   "]));
        assert_eq!(a.seed_reply_by, None);
        assert!(
            a.seed_reply_by_error.is_some(),
            "공백뿐인 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_seed_reply_by_valid_value_has_no_error() {
        let a = parse_args(s(&["--seed-request", "--seed-reply-by", "5m"]));
        assert_eq!(a.seed_reply_by.as_deref(), Some("5m"));
        assert_eq!(
            a.seed_reply_by_error, None,
            "유효한 값은 에러를 기록하지 않는다"
        );
    }

    #[test]
    fn seed_reply_by_without_request_is_rejected() {
        assert!(
            seed_reply_by_without_request_is_invalid(false, &Some("5m".to_string())),
            "seed_request=false 인데 seed_reply_by=Some → 반려"
        );
    }

    #[test]
    fn seed_reply_by_with_request_is_valid() {
        assert!(!seed_reply_by_without_request_is_invalid(
            true,
            &Some("5m".to_string())
        ));
    }

    #[test]
    fn seed_reply_by_absent_is_always_valid() {
        assert!(!seed_reply_by_without_request_is_invalid(false, &None));
        assert!(!seed_reply_by_without_request_is_invalid(true, &None));
    }

    // ── --b-task(순수 파싱 + 파일참조 해석) ───────────────────────────────────────────────────────
    #[test]
    fn parse_args_b_task_inline_text() {
        let a = parse_args(s(&["--b-task", "You are on billing. Reply in one line."]));
        assert_eq!(
            a.b_task.as_deref(),
            Some("You are on billing. Reply in one line.")
        );
    }

    #[test]
    fn parse_args_b_task_file_reference_kept_raw() {
        let a = parse_args(s(&["--b-task", "@prompts/experiments/b-task.md"]));
        assert_eq!(a.b_task.as_deref(), Some("@prompts/experiments/b-task.md"));
    }

    #[test]
    fn parse_args_b_task_next_looks_like_flag_is_an_error() {
        let a = parse_args(s(&["--b-task", "--model", "opus"]));
        assert_eq!(
            a.b_task, None,
            "다음 토큰이 플래그면 b_task 값으로 삼키지 않는다(값은 여전히 미설정)"
        );
        assert!(
            a.b_task_error.is_some(),
            "다음 토큰이 플래그처럼 보이면 오용 에러를 기록해야(F5)"
        );
        assert_eq!(
            a.model, "opus",
            "--model 은 정상 파싱돼야(에러 기록이 뒤 파싱을 막지 않는다)"
        );
    }

    #[test]
    fn parse_args_b_task_missing_value_at_end_is_an_error() {
        let a = parse_args(s(&["--b-task"]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "값 없이 끝나는 --b-task 는 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_b_task_empty_value_is_an_error() {
        let a = parse_args(s(&["--b-task", ""]));
        assert_eq!(a.b_task, None);
        assert!(a.b_task_error.is_some(), "빈 값은 오용 에러를 기록해야(F5)");
    }

    #[test]
    fn parse_args_b_task_whitespace_only_value_is_an_error() {
        let a = parse_args(s(&["--b-task", "   "]));
        assert_eq!(a.b_task, None);
        assert!(
            a.b_task_error.is_some(),
            "공백뿐인 값은 오용 에러를 기록해야(F5)"
        );
    }

    #[test]
    fn parse_args_b_task_valid_value_has_no_error() {
        let a = parse_args(s(&["--b-task", "You are on billing."]));
        assert_eq!(a.b_task.as_deref(), Some("You are on billing."));
        assert_eq!(a.b_task_error, None, "유효한 값은 에러를 기록하지 않는다");
    }

    #[test]
    fn resolve_b_task_file_path_absolute_passthrough() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let abs = if cfg!(windows) {
            "@C:\\custom\\task.md"
        } else {
            "@/custom/task.md"
        };
        let got = resolve_b_task_file_path(abs, &root).expect("@ 접두 → 파일 참조");
        let expected = if cfg!(windows) {
            "C:\\custom\\task.md"
        } else {
            "/custom/task.md"
        };
        assert_eq!(got, PathBuf::from(expected), "절대 경로는 그대로 통과");
    }

    #[test]
    fn resolve_b_task_file_path_relative_joined_under_root() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        let got = resolve_b_task_file_path("@sub/task.md", &root).expect("@ 접두 → 상대 파일 참조");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("sub/task.md") || got.ends_with("sub\\task.md"),
            "상대 경로는 repo 루트 기준 join: {got:?}"
        );
    }

    #[test]
    fn resolve_b_task_file_path_none_for_inline_text() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\repo" } else { "/repo" });
        assert_eq!(
            resolve_b_task_file_path("plain inline text", &root),
            None,
            "@ 접두 없으면 None(인라인 텍스트로 처리)"
        );
    }
}
