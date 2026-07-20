//! saturation-pilot — ADR-0090 Stage 2 컨텍스트 포화 실측 드라이버(실험 전용 bin).
//!
//! ## 역할
//! 런마다 **실 claude 에이전트 1개**(stream-json, Fresh)를 격리 임시 워크스페이스에 스폰하고, 결정적
//! 필러 문서로 컨텍스트를 목표까지 채운 뒤, **실 control 경로**(handle_send → wrap_message →
//! write_stdin_observed)로 inter-agent 메시지를 주입하고, 지연 프로브로 "지속 처리"(회상 유지)를 측정한다.
//! 관측치는 런당 JSONL 1파일로 영속한다. 순수 로직은 전부 `experiment::{cli,filler,probe,record}` 에
//! 있고 이 파일은 **thin 드라이버**(배선 + 턴 루프)다.
//!
//! ## 핵심 불변식(ADR-0090)
//! - **required-features = ["test-harness"]** — 운영 빌드는 이 bin 을 컴파일하지 않는다(릴리즈 청정).
//! - **하드 캡**: MAX_SPAWNS/MAX_TURNS/MAX_WALLCLOCK/fill clamp — 초과 시 graceful abort(코드 상수).
//! - **summary 항상 기록** — 정상/타임아웃/abort 어떤 경로든 마지막에 summary 레코드를 쓴다.
//! - **격리 워크스페이스** — fresh 임시 dir 이 cwd, 비밀 미기록, 종료 시 제거(--keep-workspace 예외).
//! - **판정 = 지연 후 회상 유지**(ADR-0088) — 즉시 ack 는 성공 아님.
//!
//! ## ★관측 경로(정직 범위)★
//! 스폰된 json 에이전트의 pump 는 decoder(ClaudeStreamDecoder)를 거쳐 **디코딩된 OutputEvent 만**
//! OutputSink 로 흘린다 — raw stream-json 라인(cache 토큰·system/init 모델 id·compact 라인)은 decoder
//! 내부에서 소비돼 사라진다(코어 무수정 제약). 두 경로로 관측을 조립한다:
//!   1. **디코딩 이벤트(항상)**: 턴 종료 = MessageDone, 토큰 = Usage(증분 input), 응답 = TextDelta 누적.
//!   2. **트랜스크립트 탭(best-effort — ADR-0090 Fix 1)**: 우리가 통제하는 세션 id(ADR-0008)로 claude 가
//!      `~/.claude/projects/<munged>/<sid>.jsonl` 에 남기는 raw 트랜스크립트를 재귀 검색해 파싱한다 →
//!      실 컨텍스트 footprint(input + cache_creation + cache_read)·정확 모델 id·event 히스토그램·compact
//!      마커. 탭이 부재하면(transcript_available=false) 하네스는 죽지 않고 문자 추정으로 폴백한다.
//! per-turn 레코드는 실측(context_tokens_real)과 추정(context_tokens_estimate)을 **둘 다** 남긴다
//! (캘리브레이션 = 파일럿 산출물). 히스토그램은 raw 트랜스크립트 타입을 우선하고, 탭 부재 시에만 디코딩
//! variant 로 폴백한다(source 필드로 명시).
// ADR-0090
// ADR-0008

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server, ManagerSlot, McpServerHandle,
};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_daemon::experiment::cli::{self, ParseError, PilotConfig};
use engram_dashboard_daemon::experiment::filler::{doc_title, filler_doc};
use engram_dashboard_daemon::experiment::probe::{
    detect_suspected_compaction, score_probe, select_detection_series, UsageSample,
};
use engram_dashboard_daemon::experiment::record::{
    cap_response, sha256_hex, CompactSignalRecord, HeaderRecord, HistogramRecord, InjectionRecord,
    ProbeRecord, Record, StallRecord, SummaryRecord, SuspectedCompactionRecord, TurnRecord,
    UsageSnapshot,
};
// ★트랜스크립트 탭(ADR-0090 Fix 1)★: 우리가 통제하는 세션 id(ADR-0008)로 claude 가 디스크에 남기는 raw
//   세션 JSONL 을 best-effort 로 읽어 실 usage(cache 항 합)·모델 id·compact 마커를 보강한다. 탭 부재는
//   하네스를 실패시키지 않는다 — 문자 추정으로 폴백. record.rs 의 raw 파서(parse_init_model/event_type_key/
//   line_mentions_compact)는 이 탭이 라인마다 호출하는 live 경로다(transcript 모듈 내부에서 재사용).
use engram_dashboard_daemon::experiment::transcript::{self, TranscriptSummary};

// ── 하드 캡(ADR-0090 불변식 — 코드 상수) ────────────────────────────────────────────
const MAX_SPAWNS_PER_INVOCATION: u32 = 6;
const MAX_TURNS_PER_RUN: u32 = 120;
const MAX_WALLCLOCK_PER_RUN: Duration = Duration::from_secs(45 * 60);
/// 턴당 대기 상한(초). 초과 시 stall 레코드 + graceful abort.
const TURN_WAIT_CAP: Duration = Duration::from_secs(240);
/// 에이전트가 목록에 나타날 때까지의 스폰 대기.
const SPAWN_APPEAR_TIMEOUT: Duration = Duration::from_secs(10);
/// 스폰 직후 트랜스크립트 파일 초기 탐색 대기(짧게). claude 는 보통 **첫 턴을 처리한 뒤에야** 트랜스크립트를
/// 쓰기 시작하므로(스모크 실측) 스폰 직후엔 대개 부재다 — 여기선 짧게만 보고, 실제 확보는 턴 루프의 lazy
/// 재검색(RunState::refresh_real_context)이 담당한다. best-effort — 못 찾아도 하네스는 실패 안 함.
// ADR-0090
const TRANSCRIPT_APPEAR_TIMEOUT: Duration = Duration::from_secs(3);
/// 헤더 작성 시 모델 id 폴링 상한(첫 턴 직후 assistant 라인 flush race 흡수). 파일은 이미 있으니 짧게.
// ADR-0090
const MODEL_RESOLVE_POLL: Duration = Duration::from_secs(4);

fn main() {
    // 프로그램명 제외 argv.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match cli::parse_args(&argv) {
        Ok(c) => c,
        Err(ParseError::Help) => {
            print!("{}", cli::usage());
            std::process::exit(0);
        }
        Err(ParseError::Unknown(f)) => {
            eprintln!("unknown flag: {f}\n");
            print!("{}", cli::usage());
            std::process::exit(2);
        }
        Err(ParseError::Invalid(msg)) => {
            eprintln!("invalid argument: {msg}\n");
            print!("{}", cli::usage());
            std::process::exit(2);
        }
    };

    // tokio 멀티스레드 런타임(MCP 서버가 async). 드라이버 본체는 blocking 로직이라 block_on 안에서 돈다.
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio 런타임 생성 실패: {e}");
            std::process::exit(1);
        }
    };

    let exit_code = rt.block_on(async { run_all(cfg).await });
    std::process::exit(exit_code);
}

/// 전 런 실행. runs·스폰 캡을 지키며 각 런을 순차 실행한다.
async fn run_all(cfg: PilotConfig) -> i32 {
    // 재현성 핀(런 전체 공통): claude 버전·git 커밋.
    let claude_version = capture_claude_version();
    let git_commit = capture_git_commit();
    if claude_version.is_none() {
        // ★skip_no_claude 이식(loud)★: claude 부재면 실험 자체가 불성립 — loud 에러 + nonzero exit.
        eprintln!(
            "FATAL [saturation-pilot]: claude CLI 를 찾을 수 없습니다(`claude --version` 실패). \
             stream-json 스폰 불가 — 실험 불성립. claude 설치/인증 확인 필요."
        );
        return 3;
    }

    // 출력 디렉토리 결정. 미지정이면 target/experiments/pilot-<UTC>.
    let out_dir = cfg
        .out
        .clone()
        .unwrap_or_else(|| default_out_dir(&utc_stamp_compact()));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("출력 디렉토리 생성 실패({}): {e}", out_dir.display());
        return 1;
    }
    eprintln!("[pilot] out dir = {}", out_dir.display());

    let runs = cfg.runs.min(MAX_SPAWNS_PER_INVOCATION);
    if runs < cfg.runs {
        eprintln!(
            "[pilot] runs {} → {} 로 클램프(MAX_SPAWNS_PER_INVOCATION)",
            cfg.runs, runs
        );
    }

    let mut worst = 0;
    for run_idx in 0..runs {
        let out_file = out_dir.join(format!("run-{run_idx}.jsonl"));
        eprintln!(
            "[pilot] === run {}/{} → {} ===",
            run_idx + 1,
            runs,
            out_file.display()
        );
        let code = run_one(
            &cfg,
            run_idx,
            &out_file,
            claude_version.clone(),
            git_commit.clone(),
        )
        .await;
        if code != 0 {
            worst = code;
        }
    }
    worst
}

/// 한 런 실행. 실패해도 summary 는 항상 쓴다(finalize). 반환 = exit 코드(0=정상).
async fn run_one(
    cfg: &PilotConfig,
    run_idx: u32,
    out_file: &std::path::Path,
    claude_version: Option<String>,
    git_commit: Option<String>,
) -> i32 {
    let run_started = Instant::now();
    let run_id = AgentId::new_v4().to_string();

    // ★finding 8 fix(순서)★: 워크스페이스를 **결과 파일보다 먼저** 만든다 — 워크스페이스 생성이 실패하면
    //   빈 결과 파일이 남지 않게(이전엔 writer 를 먼저 열어 workspace 실패 시 빈 run-N.jsonl 이 잔존).
    let workspace = std::env::temp_dir().join(format!("engram-pilot-ws-{run_id}"));
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        eprintln!("워크스페이스 생성 실패: {e}");
        return 1;
    }

    // JSONL writer(truncate — finding 10). 실패면 이 런은 무의미 — workspace 정리 후 다음 런.
    let mut writer = match JsonlWriter::create(out_file) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("JSONL 파일 생성 실패({}): {e}", out_file.display());
            let _ = std::fs::remove_dir_all(&workspace);
            return 1;
        }
    };

    let config_json = config_to_json(cfg);

    // 배선(control_send.rs wire() 미러).
    let Wiring {
        manager,
        registry,
        mcp_handle,
        data_dir,
        profile_dir,
        preset_dir,
    } = match wire(&run_id).await {
        Ok(w) => w,
        Err(e) => {
            eprintln!("배선 실패: {e}");
            // ★accepted residual(wiring 실패)★: 배선 실패는 setup abort 라 헤더 이전이다. 빈 run 파일이
            //   남으면 사후 파싱이 "0바이트 = 뭐가 잘못됐지?" 로 헷갈리므로, 최소 마커 한 줄을 남겨 원인을
            //   명시한다(over-engineer 금지 — 한 줄). 이 파일은 헤더가 없으므로 정식 레코드 스키마가 아님.
            writer.write_raw_line(r#"{"aborted":"wiring_failed"}"#);
            let _ = std::fs::remove_dir_all(&workspace);
            return 1;
        }
    };

    // 배달 관측 싱크 설치(주입 레코드용).
    let delivery_seen: Arc<Mutex<Vec<DeliveryObservation>>> = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture {
        seen: delivery_seen.clone(),
    }));

    // 실 claude(stream-json, Fresh, --model) 스폰. (헤더는 스폰 뒤에 쓴다 — 세션 id 로 트랜스크립트를
    //   먼저 찾아 resolved_model/transcript_available 을 헤더에 담기 위해.)
    let agent = match spawn_pilot_agent(&manager, &workspace, &cfg.model) {
        Some(a) => a,
        None => {
            eprintln!(
                "FATAL [saturation-pilot]: claude(stream-json) 스폰 실패 — 실험 불성립(부재/인증)."
            );
            // 스폰 실패면 세션도 트랜스크립트도 없음 — 헤더를 부재 상태로 쓰고 summary.
            writer.write(&Record::Header(HeaderRecord {
                claude_version,
                daemon_git_commit: git_commit,
                model_pin: cfg.model.clone(),
                resolved_model: None,
                resolved_model_note: Some("claude spawn failed — no session".to_string()),
                transcript_available: false,
                transcript_path: None,
                timestamp_utc: utc_stamp_rfc3339(),
                run_index: run_idx,
                run_id: run_id.clone(),
                config: config_json,
            }));
            writer.write(&Record::Summary(SummaryRecord {
                max_context_tokens: 0,
                total_turns: 0,
                duration_ms: run_started.elapsed().as_millis() as u64,
                abort_reason: Some("claude spawn failed".to_string()),
                // 스폰 실패 = 세션·트랜스크립트 없음(finding 1 필드는 부재).
                resolved_model: None,
                transcript_available: false,
                transcript_path: None,
            }));
            cleanup(
                &manager,
                None,
                mcp_handle,
                &CleanupPaths {
                    data_dir: &data_dir,
                    workspace: &workspace,
                    profile_dir: &profile_dir,
                    preset_dir: &preset_dir,
                },
                cfg,
            )
            .await;
            return 3;
        }
    };

    // ★트랜스크립트 탭 위치 확보(ADR-0090 Fix 1 / ADR-0008 경계)★: 우리가 통제하는 세션 id 를 프로필
    //   레지스트리에서 되읽어(spawn_agent 이 Fresh 에서 new_session_id 로 발급·persist), 그 sid.jsonl 을
    //   ~/.claude/projects 아래에서 재귀 검색한다. 스폰 직후엔 파일이 아직 안 생겼을 수 있어 짧게 폴링.
    //   못 찾아도(부재) best-effort — transcript_available=false 로 기록하고 문자 추정으로 폴백한다.
    let session_id = manager
        .profiles()
        .get(agent.id)
        .and_then(|p| p.claude_session_id)
        .map(|s| s.to_string());
    let transcript_path = match &session_id {
        Some(sid) => locate_transcript_with_wait(sid, TRANSCRIPT_APPEAR_TIMEOUT),
        None => None,
    };
    if let Some(tp) = &transcript_path {
        eprintln!("[pilot] transcript tap: {}", tp.display());
    } else {
        eprintln!(
            "[pilot] transcript tap 부재(sid={:?}) — 문자 추정으로 폴백(best-effort)",
            session_id
        );
    }

    // 출력 관측 sink 부착(턴 종료·usage·응답텍스트·compact 스캔).
    let obs = Arc::new(TurnObserver::new());
    let sink_id = match manager.subscribe(agent.id, obs.clone()) {
        Ok(id) => Some(id),
        Err(e) => {
            eprintln!("구독 실패: {e}");
            None
        }
    };

    // ── 런 상태 ── (트랜스크립트 탭 경로 + 세션 id 를 넘긴다. path 가 아직 없어도 턴 진행 중 lazy
    //   재검색으로 붙는다 — claude 는 첫 턴 처리 후에야 트랜스크립트를 쓰기 시작할 수 있어서.)
    let mut state = RunState::new(transcript_path.clone(), session_id.clone());

    // ★finding 10 fix — 헤더-first 계약★: HeaderRecord 를 **파일의 첫 줄**로 쓴다(이전엔 첫 task 턴 뒤에
    //   써서 turn 이 헤더보다 앞섰다). 스폰 직후엔 트랜스크립트가 아직 없어 resolved_model 이 대개 None
    //   이지만(note 로 명시), 헤더-first 계약(파일 첫 줄 = header)이 우선이다. 정확 모델 id 는 런 끝
    //   authoritative 파싱(final_transcript.resolved_model)이 별도로 남긴다.
    {
        let resolved_model = state
            .transcript_path
            .as_deref()
            .and_then(|p| poll_resolved_model(p, MODEL_RESOLVE_POLL));
        let (note, available, path_str) = match (&state.transcript_path, &resolved_model) {
            (Some(p), Some(_)) => (None, true, Some(p.display().to_string())),
            (Some(p), None) => (
                Some("트랜스크립트는 찾았으나 아직 모델 라인 미기록(스폰 시점) — 런 끝 재파싱으로 대조 가능".to_string()),
                true,
                Some(p.display().to_string()),
            ),
            (None, _) => (
                Some("트랜스크립트 부재(스폰 시점 — 첫 턴 후 나타날 수 있음) — 실 usage·모델 id 는 런 끝 재파싱으로 확정, 진행은 문자 추정 폴백".to_string()),
                false,
                None,
            ),
        };
        writer.write(&Record::Header(HeaderRecord {
            claude_version,
            daemon_git_commit: git_commit,
            model_pin: cfg.model.clone(),
            resolved_model,
            resolved_model_note: note,
            transcript_available: available,
            transcript_path: path_str,
            timestamp_utc: utc_stamp_rfc3339(),
            run_index: run_idx,
            run_id: run_id.clone(),
            config: config_json,
        }));
    }

    // ★finding 8 fix — 패닉을 포함한 모든 경로에서 cleanup 보장(RAII/catch_unwind)★: 동기 런 본체
    //   (턴 루프 + 파이널라이즈)를 catch_unwind 로 감싼다. 본체가 패닉해도 아래 cleanup(kill agent +
    //   워크스페이스/temp 제거 + MCP 종료)은 반드시 실행된다 — 이전엔 mid-run 패닉이 유일한 cleanup
    //   호출을 건너뛰어 claude 가 살아남고 temp dir 이 남았다. state/writer 는 본체가 소유(move)하되,
    //   패닉해도 outcome 만 잃고 정리 리소스(manager/mcp_handle/paths)는 이 스코프에 남아 회수된다.
    let run_ctx = RunDriveCtx {
        manager: &manager,
        registry: &registry,
        agent: &agent,
        obs: &obs,
        delivery_seen: &delivery_seen,
        run_id: &run_id,
        run_started,
        cfg,
    };
    let drive_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive_run(&run_ctx, &mut state, &mut writer)
    }));
    let (abort_reason, total_turns, max_ctx) = match drive_result {
        Ok(outcome) => outcome,
        Err(_) => {
            eprintln!("[pilot] run {run_idx} PANICKED — cleanup 강제 실행(finding 8)");
            // 패닉해도 summary 를 남긴다(항상 기록 불변식) — writer 는 catch_unwind 밖에서 여전히 유효.
            //   ★finding 1★: 패닉 경로도 best-effort 로 트랜스크립트를 한 번 파싱해 resolved_model 을 실어
            //   재현성 핀을 최대한 보존한다(탭 부재/파싱 실패면 None).
            let panic_ts = state
                .transcript_path
                .as_deref()
                .and_then(transcript::parse_transcript);
            writer.write(&Record::Summary(SummaryRecord {
                max_context_tokens: state.max_context_tokens,
                total_turns: state.turn_idx,
                duration_ms: run_started.elapsed().as_millis() as u64,
                abort_reason: Some("run body panicked".to_string()),
                resolved_model: panic_ts.as_ref().and_then(|ts| ts.resolved_model.clone()),
                transcript_available: state.transcript_path.is_some(),
                transcript_path: state
                    .transcript_path
                    .as_deref()
                    .map(|p| p.display().to_string()),
            }));
            (
                Some("run body panicked".to_string()),
                state.turn_idx,
                state.max_context_tokens,
            )
        }
    };

    // cleanup: 구독 해제 → 에이전트 kill → MCP 종료 → data_dir/워크스페이스/profile·preset temp 제거.
    //   ★finding 8/9★: catch_unwind 밖이라 패닉 시에도 반드시 실행되고, profile/preset temp 도 지운다.
    if let Some(sid) = sink_id {
        let _ = manager.unsubscribe(agent.id, sid);
    }
    cleanup(
        &manager,
        Some(agent.id),
        mcp_handle,
        &CleanupPaths {
            data_dir: &data_dir,
            workspace: &workspace,
            profile_dir: &profile_dir,
            preset_dir: &preset_dir,
        },
        cfg,
    )
    .await;

    if abort_reason.is_some() {
        eprintln!("[pilot] run {run_idx} aborted: {abort_reason:?}");
        // abort 는 graceful — nonzero 로 상위에 알리되 파일은 온전.
        return 4;
    }
    eprintln!("[pilot] run {run_idx} done: turns={total_turns} max_ctx={max_ctx}");
    0
}

// ═══════════════════════════════════════════════════════════════════════════════════
// drive_run — 동기 런 본체(턴 루프 + 파이널라이즈). catch_unwind 로 감싸 패닉해도 cleanup 이 돌게 한다.
// ═══════════════════════════════════════════════════════════════════════════════════

/// drive_run 이 빌려 쓰는 런 컨텍스트(불변 리소스 참조 묶음). state/writer 는 별도 &mut 로 받는다.
struct RunDriveCtx<'a> {
    manager: &'a Arc<AgentManager>,
    registry: &'a Arc<ControlRegistry>,
    agent: &'a AgentInfo,
    obs: &'a Arc<TurnObserver>,
    delivery_seen: &'a Arc<Mutex<Vec<DeliveryObservation>>>,
    run_id: &'a str,
    run_started: Instant,
    cfg: &'a PilotConfig,
}

/// 하드 캡 게이트 결과 — 계속 진행 가능(Ok) 또는 캡에 걸려 중단(사유).
/// ★finding 5/6★: 모든 턴(주입·프로브·compact·FINAL 포함) **직전**에 이 게이트를 통과해야 한다.
fn cap_gate(ctx: &RunDriveCtx, state: &RunState) -> Result<(), String> {
    // wallclock 캡 — 어떤 phase(루프 안/후) 진입 전에도 검사(finding 6).
    if ctx.run_started.elapsed() >= MAX_WALLCLOCK_PER_RUN {
        return Err("MAX_WALLCLOCK_PER_RUN reached".to_string());
    }
    // turn 캡 — 주입/compact/FINAL 도 turn_idx 를 올리므로 모든 턴 직전 검사(finding 5).
    if state.turn_idx >= MAX_TURNS_PER_RUN {
        return Err("MAX_TURNS_PER_RUN reached".to_string());
    }
    Ok(())
}

/// 동기 런 본체. 반환 = (abort_reason, total_turns, max_context_tokens).
///
/// ★불변식(finding 1/2/5/6)★: **모든 stdin-생성 write 는 그 직전에 cap_gate 를 통과하고, 직후 자기 턴의
///   wait_turn_end 로 펜싱된다** — 두 write 사이에 반드시 wait_turn_end 가 끼어야 응답 오귀속이 안 난다.
///   주입도 첫급 턴(begin_turn → inject → wait_turn_end → TurnRecord(kind=inject) → turn_idx++)이다.
fn drive_run(
    ctx: &RunDriveCtx,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> (Option<String>, u32, u64) {
    let RunDriveCtx {
        manager,
        registry,
        agent,
        obs,
        delivery_seen,
        run_id,
        cfg,
        ..
    } = *ctx;
    let mut abort_reason: Option<String> = None;

    // Turn 1 = 원과제 지시. (drive_turn 이 lazy 재검색으로 트랜스크립트를 붙이고 실 usage 를 채운다.)
    //   ★finding 5/6★: 첫 턴도 cap_gate 통과 후에만.
    if let Err(r) = cap_gate(ctx, state) {
        abort_reason = Some(r);
    } else {
        let task_prompt = original_task_prompt();
        match drive_turn(
            manager,
            agent.id,
            obs,
            &task_prompt,
            "task",
            0,
            state,
            writer,
        ) {
            TurnResult::Ok => {}
            TurnResult::Stalled => abort_reason = Some("turn 1 (task) stalled".to_string()),
            TurnResult::Terminal => {
                abort_reason = Some("agent terminated during task turn".to_string())
            }
            TurnResult::Error(e) => abort_reason = Some(format!("turn 1 (task) error: {e}")),
        }
    }

    // 주입 스케줄: inject_at 분율 → 목표 토큰 대비 문턱.
    let inject_thresholds: Vec<(u32, f64, u64)> = cfg
        .inject_at
        .iter()
        .enumerate()
        .map(|(k, &frac)| {
            (
                k as u32,
                frac,
                (cfg.fill_target_tokens as f64 * frac) as u64,
            )
        })
        .collect();
    let mut next_inject = 0usize; // 다음에 발화할 주입 인덱스.
    let mut pending_probes: Vec<PendingProbe> = Vec::new();

    // ── Fill + Inject + Probe 루프 ──
    if abort_reason.is_none() {
        loop {
            // 하드 캡 체크(finding 5/6 — 모든 턴 직전 단일 게이트).
            if let Err(r) = cap_gate(ctx, state) {
                abort_reason = Some(r);
                break;
            }

            // 주입 발화: 현재 컨텍스트가 다음 주입 문턱을 넘었으면 주입.
            if next_inject < inject_thresholds.len() {
                let (k, frac, threshold) = inject_thresholds[next_inject];
                if state.max_context_tokens >= threshold {
                    // ★finding 1 fix★: 주입은 이제 첫급 턴이다 — do_injection 이 begin_turn → inject →
                    //   wait_turn_end(baseline) → TurnRecord(kind=inject) → turn_idx++ 를 수행한다.
                    //   그 안에서 실패/스톨하면 abort 시그널을 돌려준다.
                    match do_injection(
                        manager,
                        registry,
                        obs,
                        agent.id,
                        agent.epoch,
                        run_id,
                        k,
                        frac,
                        cfg.seed,
                        delivery_seen,
                        state,
                        writer,
                    ) {
                        InjectOutcome::Ok(inj) => {
                            pending_probes.push(PendingProbe {
                                k,
                                remaining_gap: cfg.probe_gap_turns,
                                sender_name: inj.sender_name,
                                msg_id: inj.msg_id,
                                codeword: inj.codeword,
                            });
                            next_inject += 1;
                            // ★finding 5 fix — 불변식: probe gap 은 FILL 턴만 센다★. 이전엔 여기서(주입 턴
                            //   직후) 모든 대기 프로브의 gap 을 깎아, 방금 넣은 주입 턴 자신과 **다른 주입들**
                            //   까지 gap 을 소비했다(--probe-gap-turns 정의 = "주입 후 fill 턴 수" 와 어긋남).
                            //   이제 주입 턴은 gap 을 소비하지 않는다 — gap 감소는 fill 턴 처리 지점 **한 곳**
                            //   에서만 일어난다(아래 fill 턴 뒤). 방금 push 한 프로브의 gap 은 온전히 fill 턴
                            //   개수로만 카운트다운된다.
                            continue;
                        }
                        InjectOutcome::Abort(r) => {
                            abort_reason = Some(r);
                            break;
                        }
                    }
                }
            }

            // 지연 프로브 발화: gap 이 0 이 된 프로브를 실행.
            if let Some(pos) = pending_probes.iter().position(|p| p.remaining_gap == 0) {
                let probe = pending_probes.remove(pos);
                if let Err(r) = run_probe(manager, agent.id, obs, &probe, state, writer) {
                    abort_reason = Some(r);
                    break;
                }
                continue;
            }

            // 포화 도달 + 모든 주입/프로브 소진 → fill 루프 종료.
            let fill_target_reached = state.max_context_tokens >= cfg.fill_target_tokens;
            if fill_target_reached
                && next_inject >= inject_thresholds.len()
                && pending_probes.is_empty()
            {
                break;
            }

            // fill 턴 1개 — 다음 doc 을 보낸다.
            state.doc_counter += 1;
            let doc_n = state.doc_counter;
            let body = filler_doc(cfg.seed, doc_n, cfg.doc_chars);
            let prompt = format!("{body}\nreceived {doc_n}?");
            match drive_turn(
                manager, agent.id, obs, &prompt, "fill", doc_n, state, writer,
            ) {
                TurnResult::Ok => {}
                TurnResult::Stalled => {
                    abort_reason = Some(format!("fill turn (doc {doc_n}) stalled"));
                    break;
                }
                TurnResult::Terminal => {
                    abort_reason = Some("agent terminated during fill".to_string());
                    break;
                }
                TurnResult::Error(e) => {
                    abort_reason = Some(format!("fill turn (doc {doc_n}) error: {e}"));
                    break;
                }
            }
            // ★finding 5 불변식 — probe gap 감소는 오직 여기(fill 턴 처리 직후) 한 곳★: --probe-gap-turns 는
            //   "주입 후 몇 개의 FILL 턴을 지나서 프로브를 낼지" 다. 주입 턴·다른 주입은 gap 을 소비하지
            //   않는다(위 주입 갈래에서 감소 코드를 제거함). 그래서 gap=N 이면 정확히 N 개 fill 턴 뒤 프로브가
            //   발화한다(프로브 발화 체크가 fill 처리보다 루프 앞에 있어, gap 0 도달 다음 iteration 에 발화).
            for p in pending_probes.iter_mut() {
                if p.remaining_gap > 0 {
                    p.remaining_gap -= 1;
                }
            }
        }
    }

    // 남은 대기 프로브 전부 소진(gap 여부 무관 — 런 끝에 회상 측정). ★finding 5/6★: 각 프로브 직전
    //   cap_gate — 후처리 phase 도 캡 밖으로 새지 않게.
    if abort_reason.is_none() {
        let leftover: Vec<PendingProbe> = std::mem::take(&mut pending_probes);
        for probe in leftover {
            if let Err(r) = cap_gate(ctx, state) {
                abort_reason = Some(r);
                break;
            }
            if let Err(r) = run_probe(manager, agent.id, obs, &probe, state, writer) {
                abort_reason = Some(r);
                break;
            }
        }
    }

    // ★finding 3 fix — 강제 /compact phase 제거★: 이전엔 포화 도달 시 리터럴 `/compact` 를 평범한 유저
    //   TEXT 로 보내 반응을 관측하는 phase 가 있었으나, stream-json headless 에는 대화형 슬래시 인터셉트가
    //   없어 `/compact` 는 native compaction 을 **트리거하지 못하는** 평문일 뿐이라 오해를 부르는 관측이었다.
    //   그래서 phase 전체를 삭제한다. compaction 관측은 **organic native 압축**을 트랜스크립트 compact-marker
    //   캡처(transcript.compact_marker_lines)로만 잡는다 — 스모크가 organic 압축이 실제로 일어나고 캡처됨을
    //   증명했다(런 끝 authoritative 파싱 경로가 이미 그 마커를 CompactSignal 로 기록한다).

    // FINAL REPORT 프로브(원과제 완료). ★finding 5/6★: cap_gate 통과 후에만.
    if abort_reason.is_none() {
        match cap_gate(ctx, state) {
            Ok(()) => {
                let final_probe = PendingProbe {
                    k: u32::MAX, // 표식: FINAL(주입 없음).
                    remaining_gap: 0,
                    sender_name: String::new(),
                    msg_id: String::new(),
                    codeword: String::new(),
                };
                if let Err(r) = run_final_report(
                    manager,
                    agent.id,
                    obs,
                    &final_probe,
                    cfg,
                    state.doc_counter,
                    state,
                    writer,
                ) {
                    abort_reason = Some(r);
                }
            }
            Err(r) => abort_reason = Some(r),
        }
    }

    // ★런 끝 authoritative 트랜스크립트 파싱(ADR-0090 Fix 1)★: 전체 트랜스크립트를 한 번에 접어 raw event
    //   히스토그램·compact 마커·실 usage 계열을 확정한다(턴별 best-effort 탭보다 이게 최종 진실). 탭 부재면
    //   None → 디코딩 variant 히스토그램으로 폴백.
    let final_transcript: Option<TranscriptSummary> = state
        .transcript_path
        .as_deref()
        .and_then(transcript::parse_transcript);

    // event 히스토그램 — 트랜스크립트가 있으면 raw 타입 히스토그램(authoritative), 없으면 디코딩 variant.
    match &final_transcript {
        Some(ts) => writer.write(&Record::Histogram(HistogramRecord {
            counts: ts.event_histogram.clone(),
            source: "transcript_raw_stream_json_types (session JSONL tap — ADR-0090 Fix 1)"
                .to_string(),
        })),
        None => writer.write(&Record::Histogram(HistogramRecord {
            counts: obs.histogram_snapshot(),
            source: "decoded_output_event_variants (transcript tap absent — fallback)".to_string(),
        })),
    }

    // ★finding 2 fix — 단일 일관 계열에서만 감지★: authoritative real_usage_series(트랜스크립트, 순수
    //   실측)가 있으면 그 위에서, 없으면 estimate_samples(순수 추정) 위에서 감지한다. 두 계열 다 단일
    //   소스라 소스 전환(추정→실측) 지점의 인공 급감이 원천적으로 없다 — 이전엔 turn마다 실측/추정을 섞은
    //   계열을 써서 탭이 처음 붙는 턴에 가짜 compaction 이 섰다. 선택 로직(never mix)은 순수 함수
    //   select_detection_series 로 내려 단위 테스트가 직접 커버한다.
    let real_footprints: Option<Vec<u64>> = final_transcript.as_ref().and_then(|ts| {
        if ts.real_usage_series.is_empty() {
            None
        } else {
            Some(
                ts.real_usage_series
                    .iter()
                    .map(|u| u.context_footprint())
                    .collect(),
            )
        }
    });
    let detection_series =
        select_detection_series(real_footprints.as_deref(), &state.estimate_samples);
    let flags = detect_suspected_compaction(&detection_series);
    if !flags.is_empty() {
        writer.write(&Record::SuspectedCompaction(SuspectedCompactionRecord {
            flagged_turn_idxs: flags,
        }));
    }

    // compact 마커 — 트랜스크립트에서 잡은 verbatim 라인(authoritative)을 우선 기록.
    if let Some(ts) = &final_transcript {
        for line in &ts.compact_marker_lines {
            writer.write(&Record::CompactSignal(CompactSignalRecord {
                verbatim: cap_response(line),
                source: "transcript_compact_marker".to_string(),
            }));
        }
    }
    // 디코딩 경로에서 스캔한 compact 근사 신호(Structured/Error)도 함께 기록(보완).
    for sig in obs.drain_compact_signals() {
        writer.write(&Record::CompactSignal(sig));
    }

    // summary(항상 기록). max_context_tokens = 실측 최대(트랜스크립트 있으면) 우선, 없으면 문자 추정 최대.
    let max_real = final_transcript.as_ref().and_then(|ts| {
        ts.real_usage_series
            .iter()
            .map(|u| u.context_footprint())
            .max()
    });
    let max_ctx = max_real.unwrap_or(state.max_context_tokens);
    // ★finding 8★: writer io_errors 를 abort_reason 에 반영(조용한 기록 손실 가시화).
    if writer.io_errors > 0 && abort_reason.is_none() {
        abort_reason = Some(format!("{} JSONL write/flush errors", writer.io_errors));
    }
    // ★finding 1 fix — summary 가 authoritative resolved_model 을 실어 재현성 핀 보존★: 헤더의
    //   resolved_model 은 스폰 직후(트랜스크립트 미기록)라 대개 None 이었다. 런 끝 authoritative 파싱이
    //   확정한 모델 id·탭 존재·경로를 summary 에 담아 헤더만 보면 유실되던 핀(ADR-0088 d5a)을 복구한다.
    let summary_resolved_model = final_transcript
        .as_ref()
        .and_then(|ts| ts.resolved_model.clone());
    let summary_transcript_path = state
        .transcript_path
        .as_deref()
        .map(|p| p.display().to_string());
    writer.write(&Record::Summary(SummaryRecord {
        max_context_tokens: max_ctx,
        total_turns: state.turn_idx,
        duration_ms: ctx.run_started.elapsed().as_millis() as u64,
        abort_reason: abort_reason.clone(),
        resolved_model: summary_resolved_model,
        transcript_available: final_transcript.is_some(),
        transcript_path: summary_transcript_path,
    }));

    (abort_reason, state.turn_idx, max_ctx)
}

// ═══════════════════════════════════════════════════════════════════════════════════
// 배선 (control_send.rs wire() 미러)
// ═══════════════════════════════════════════════════════════════════════════════════

struct Wiring {
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    mcp_handle: McpServerHandle,
    data_dir: PathBuf,
    /// ★finding 9★: per-run profile/preset 임시 dir(cleanup 이 이것도 제거해야 함 — 이전엔 누수).
    profile_dir: PathBuf,
    preset_dir: PathBuf,
}

/// 실 DaemonControlChannel + MCP 서버 + AgentManager 배선(control_send.rs wire() 순서 미러).
async fn wire(tag: &str) -> Result<Wiring, String> {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let handle = start_mcp_server(registry.clone(), slot.clone())
        .await
        .map_err(|e| format!("start mcp server: {e}"))?;
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-pilot-{tag}"));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url,
        data_dir.clone(),
        None, // send_exe: 파일럿은 handle_send 직접 호출이라 CLI 경로 불요.
        // ADR-0092: 파일럿은 프라이밍 무관(주입 확립은 priming_smoke bin) — Noop 으로 오늘 동작 불변.
        Arc::new(engram_dashboard_daemon::control::priming::NoopPrimingProvider),
    ));

    let sink: Arc<dyn StatusSink> = Arc::new(NoopStatus);
    // ★finding 9★: profile/preset 임시 dir 경로를 Wiring 으로 넘겨 cleanup 이 제거하게 한다(누수 방지).
    let profile_dir = std::env::temp_dir().join(format!("engram-pilot-prof-{tag}"));
    let preset_dir = std::env::temp_dir().join(format!("engram-pilot-preset-{tag}"));
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

    Ok(Wiring {
        manager,
        registry,
        mcp_handle: handle,
        data_dir,
        profile_dir,
        preset_dir,
    })
}

/// 실 claude(stream-json, Fresh, --model)를 워크스페이스 cwd 로 스폰. control_send.rs spawn_json_agent 미러.
fn spawn_pilot_agent(
    manager: &Arc<AgentManager>,
    workspace: &std::path::Path,
    model: &str,
) -> Option<AgentInfo> {
    let profile = AgentProfile::new(
        format!("pilot-{}", &AgentId::new_v4().to_string()[..8]),
        AgentCommand::Claude {
            // ★모델 핀★: extra_args 로 --model 주입(백엔드 코드 무변경 — ADR-0090 d3).
            extra_args: vec!["--model".to_string(), model.to_string()],
            output_format: ClaudeOutputFormat::StreamJson,
        },
        workspace.to_path_buf(),
        vec![],
        false,
    );
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

/// 세션 id 로 트랜스크립트 파일을 재귀 검색하되, 아직 안 생겼으면 timeout 까지 폴링한다. claude 는 첫
/// stream-json 라인을 처리한 뒤에야 트랜스크립트를 쓰기 시작할 수 있어(스폰 직후엔 부재) 잠깐 기다린다.
/// best-effort: timeout 내 못 찾으면 None(문자 추정 폴백).
// ADR-0090 ADR-0008
fn locate_transcript_with_wait(session_id: &str, timeout: Duration) -> Option<PathBuf> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(path) = transcript::locate_transcript(session_id) {
            return Some(path);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 트랜스크립트를 재파싱해 모델 id 가 나올 때까지 timeout 까지 폴링한다(assistant.message.model flush race
/// 흡수). 파일은 이미 존재하는 상태에서 부른다. best-effort: 못 얻으면 None. ★탭이 하네스를 막지 않는다★.
// ADR-0090
fn poll_resolved_model(path: &std::path::Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(model) = transcript::parse_transcript(path).and_then(|s| s.resolved_model) {
            return Some(model);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════
// 출력 관측 sink — 턴 종료·usage·응답 텍스트·compact 스캔
// ═══════════════════════════════════════════════════════════════════════════════════

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// 배달 관측 캡처(주입 레코드용).
struct DeliveryCapture {
    seen: Arc<Mutex<Vec<DeliveryObservation>>>,
}
impl DeliveryObserver for DeliveryCapture {
    fn observe(&self, obs: DeliveryObservation) {
        self.seen.lock().unwrap().push(obs);
    }
}

/// 턴 관측기 — OutputSink 로 온 디코딩 이벤트를 소비한다.
///   - MessageDone → done_count 증가(턴 종료 신호). Condvar 로 대기자를 깨운다.
///   - Usage → 마지막 usage 갱신.
///   - TextDelta → 현재 턴 응답 버퍼에 누적.
///   - Structured/Error → event 히스토그램 + "compact" 문자열 스캔.
struct TurnObserver {
    id: SinkId,
    inner: Mutex<ObserverInner>,
    /// MessageDone 카운트(턴 종료 신호) — Condvar 대기의 조건.
    done_count: AtomicU64,
    cv: Condvar,
    /// 터미널 상태 도달(에이전트 죽음) 감지 — status sink 가 아니라 여기선 미사용(별도 list 체크).
    _reserved: (),
}

#[derive(Default)]
struct ObserverInner {
    /// 현재 턴 응답 텍스트 누적(TextDelta).
    response_buf: String,
    /// 최근 usage(input/output).
    last_usage: Option<(u64, u64)>,
    /// 디코딩된 이벤트 variant 히스토그램.
    histogram: BTreeMap<String, u64>,
    /// compact 근사 신호(Structured/Error 텍스트에서 "compact" 발견).
    compact_signals: Vec<CompactSignalRecord>,
    /// ★finding 3★: 현재 턴에서 관측된 **비-compaction** API 에러 메시지(있으면). MessageDone 이 뒤따라도
    ///   이 턴은 실패로 봐야 한다(abort_reason:null 로 실패 은폐 방지). begin_turn 에서 리셋.
    turn_error: Option<String>,
}

impl TurnObserver {
    fn new() -> Self {
        Self {
            id: SinkId::new_v4(),
            inner: Mutex::new(ObserverInner::default()),
            done_count: AtomicU64::new(0),
            cv: Condvar::new(),
            _reserved: (),
        }
    }

    /// 새 턴 시작 — 응답 버퍼·usage·턴 에러를 리셋한다(이전 턴 누적 제거).
    ///
    /// ★load-bearing(finding 1/2)★: **caller MUST wait_turn_end before the next stdin write** — begin_turn
    ///   으로 리셋한 뒤 stdin write 를 하고 그 턴의 wait_turn_end 로 펜싱해야 한다. 펜싱 없이 다음 stdin
    ///   write 를 하면 이전 턴의 늦은 MessageDone 이 다음 wait 를 조기 해제해 응답이 엉뚱한 턴에 귀속된다.
    fn begin_turn(&self) {
        let mut g = self.inner.lock().unwrap();
        g.response_buf.clear();
        g.last_usage = None;
        g.turn_error = None;
    }

    /// 현재 턴에서 관측된 비-compaction API 에러(있으면). MessageDone 후에도 이게 Some 이면 턴 실패.
    fn turn_error(&self) -> Option<String> {
        self.inner.lock().unwrap().turn_error.clone()
    }

    /// 현재 done_count 스냅샷(턴 시작 직전에 잡아 두고, 이보다 커지면 턴 종료).
    fn done_snapshot(&self) -> u64 {
        self.done_count.load(Ordering::Acquire)
    }

    /// done_count 가 `baseline` 을 초과할 때까지(=이번 턴의 MessageDone) 대기. 타임아웃이면 false.
    fn wait_turn_end(&self, baseline: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let guard = self.inner.lock().unwrap();
        // done_count 는 atomic 이라 lock 밖에서 바뀌지만, Condvar 대기는 이 lock 을 놓고 자므로
        //   notify 를 놓치지 않게 wait_timeout 루프로 확인한다(spurious wake 방어).
        let mut g = guard;
        loop {
            if self.done_count.load(Ordering::Acquire) > baseline {
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

    /// 현재 턴 응답 텍스트 스냅샷.
    fn response_text(&self) -> String {
        self.inner.lock().unwrap().response_buf.clone()
    }

    /// 최근 usage 스냅샷.
    fn last_usage(&self) -> Option<(u64, u64)> {
        self.inner.lock().unwrap().last_usage
    }

    fn histogram_snapshot(&self) -> BTreeMap<String, u64> {
        self.inner.lock().unwrap().histogram.clone()
    }

    fn drain_compact_signals(&self) -> Vec<CompactSignalRecord> {
        std::mem::take(&mut self.inner.lock().unwrap().compact_signals)
    }
}

impl OutputSink for TurnObserver {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        let OutputPayload::Event(ev) = frame.payload else {
            // json 에이전트는 Event 만 오지만(TerminalBytes 아님), 방어적으로 Bytes 는 무시.
            return Ok(());
        };
        let mut g = self.inner.lock().unwrap();
        // 히스토그램 키 = 디코딩된 variant 이름(raw 타입 아님 — honest scope).
        let key = decoded_variant_key(ev);
        *g.histogram.entry(key).or_insert(0) += 1;

        match ev {
            OutputEvent::TextDelta { text, .. } => {
                g.response_buf.push_str(text);
            }
            OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                g.last_usage = Some((*input_tokens, *output_tokens));
            }
            OutputEvent::MessageDone { .. } => {
                // 턴 종료 — lock 밖 atomic 증가 후 대기자 깨움. lock 보유 중이지만 짧다.
                self.done_count.fetch_add(1, Ordering::Release);
                self.cv.notify_all();
            }
            OutputEvent::Structured { json, .. } => {
                // compact 근사 스캔(best-effort — raw 라인 아님).
                if json.to_ascii_lowercase().contains("compact") {
                    g.compact_signals.push(CompactSignalRecord {
                        verbatim: cap_response(json),
                        source: "decoded_structured".to_string(),
                    });
                }
            }
            OutputEvent::Error(msg) => {
                // ★finding 3(substring) fix★: OutputEvent::Error 는 텍스트에 "compact" 가 들어 있어도
                //   **항상 실 에러**다 — substring 이 에러를 무해 신호로 강등하지 못한다. 이전엔 "compact"
                //   포함 시 turn_error 를 안 세우고 compact 신호로만 기록해, "compaction failed" 류 실 API
                //   에러가 성공(abort_reason:null)으로 삼켜졌다. 이제: 첫 에러를 turn_error 로 마킹(턴 실패)
                //   하고, "compact" 를 언급하면 진단용 compact 신호로도 **함께** 기록한다(강등 아닌 병기).
                if g.turn_error.is_none() {
                    g.turn_error = Some(cap_response(msg));
                }
                if msg.to_ascii_lowercase().contains("compact") {
                    g.compact_signals.push(CompactSignalRecord {
                        verbatim: cap_response(msg),
                        source: "decoded_error (real error; mentions compact — logged for diagnosis, not downgraded)".to_string(),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

/// 디코딩된 OutputEvent → 히스토그램 키(variant 이름). raw stream-json 타입이 아님(honest scope).
fn decoded_variant_key(ev: &OutputEvent) -> String {
    match ev {
        OutputEvent::TerminalBytes(_) => "TerminalBytes",
        OutputEvent::TextDelta { .. } => "TextDelta",
        OutputEvent::ToolCall { .. } => "ToolCall",
        OutputEvent::Usage { .. } => "Usage",
        OutputEvent::MessageDone { .. } => "MessageDone",
        OutputEvent::Error(_) => "Error",
        OutputEvent::Structured { kind, .. } => return format!("Structured/{kind}"),
    }
    .to_string()
}

// ═══════════════════════════════════════════════════════════════════════════════════
// 턴 드라이버
// ═══════════════════════════════════════════════════════════════════════════════════

enum TurnResult {
    Ok,
    Stalled,
    Terminal,
    /// ★finding 3★: 턴은 MessageDone 으로 끝났으나 그 사이 비-compaction API 에러가 관측됐다 — 실패.
    Error(String),
}

/// 문자→토큰 근사 비율(영문 대략 4 char/token). fill 진행 추정에만 쓰는 heuristic.
const CHARS_PER_TOKEN_EST: u64 = 4;

/// 런 진행 상태.
struct RunState {
    /// 지금까지 진행한 총 턴 수.
    turn_idx: u32,
    /// fill 진행/주입 문턱 판정 기준(= 우리가 보낸 누적 문자의 토큰 추정 — 아래 ★파일럿 발견★). 트랜스크립트
    ///   탭이 있어도 **진행 제어는 이 추정으로 한다**(우리가 통제·결정적이라 스케줄이 재현 가능). 실측은
    ///   레코드에 나란히 남겨 사후 캘리브레이션한다.
    max_context_tokens: u64,
    /// fill doc 카운터.
    doc_counter: u32,
    /// ★finding 2 fix★: **추정(estimate)-only** 감지 계열. 이전엔 턴마다 "실측 있으면 실측, 없으면 추정" 을
    ///   섞어 담아, 트랜스크립트 탭이 처음 붙는 순간 소스 전환(추정→실측)으로 값이 뚝 떨어져 가짜 compaction
    ///   플래그가 섰다. 이제 이 계열은 **순수 추정만** 담고(소스 혼합 없음), 트랜스크립트 탭이 있으면
    ///   finalize 가 감지를 real_usage_series(순수 실측) 위에서 돌린다 — 둘 다 단일 소스라 인공 급감 불가.
    estimate_samples: Vec<UsageSample>,
    /// ★파일럿 발견(2026-07-20 스모크 실측)★: 스폰 경로의 디코딩된 `Usage.input_tokens` 는 그 턴의
    ///   **증분 입력**(≈3)만 보고하고 **누적 컨텍스트가 아니다**. 실제 컨텍스트 크기는 트랜스크립트의
    ///   `input + cache_creation_input_tokens + cache_read_input_tokens` 인데, core decoder 가 cache
    ///   필드를 버려(input/output 만 추출) 스폰 경로에선 못 얻는다(코어 무수정 제약). 그래서 fill 진행/
    ///   주입 문턱 판정은 usage 가 아니라 **우리가 보낸 누적 문자수의 토큰 추정**으로 한다(우리가 통제·
    ///   결정적). 실 컨텍스트는 트랜스크립트 탭(Fix 1)이 채운다 — 그러나 스케줄 제어는 추정 유지.
    cumulative_chars_sent: u64,
    /// 트랜스크립트 탭 경로(있으면). 턴마다 이 파일을 재파싱해 **가장 최신 실 usage** 를 뽑는다(best-effort).
    ///   ADR-0008 경계: 이건 측정 탭일 뿐 — 부재해도 하네스는 정상 동작한다(문자 추정 폴백).
    transcript_path: Option<PathBuf>,
    /// 통제 세션 id — transcript_path 가 아직 None 이면 refresh 때 **지연 재검색**에 쓴다. claude 는
    ///   스폰 직후가 아니라 첫 턴을 처리한 뒤에야 트랜스크립트를 쓰기 시작할 수 있어(스모크 실측), 초기
    ///   locate 가 실패해도 턴이 진행되면 파일이 나타난다 — 그래서 lazy 재검색이 필요하다.
    session_id: Option<String>,
    /// 관측된 최신 실 컨텍스트 footprint(트랜스크립트 탭이 마지막으로 준 값 — None 이면 아직/영영 부재).
    latest_real_context: Option<u64>,
}
impl RunState {
    fn new(transcript_path: Option<PathBuf>, session_id: Option<String>) -> Self {
        Self {
            turn_idx: 0,
            max_context_tokens: 0,
            doc_counter: 0,
            estimate_samples: Vec::new(),
            cumulative_chars_sent: 0,
            transcript_path,
            session_id,
            latest_real_context: None,
        }
    }

    /// 보낸 프롬프트 문자수를 누적하고, 그 토큰 추정을 max_context_tokens 로 반영한다(fill 진행 신호).
    /// ★왜 usage 가 아니라 여기★: 위 필드 주석 — 디코딩된 usage 는 누적 컨텍스트를 반영 못 한다.
    fn account_context(&mut self, prompt_len: usize) {
        self.cumulative_chars_sent += prompt_len as u64;
        let est = self.cumulative_chars_sent / CHARS_PER_TOKEN_EST;
        if est > self.max_context_tokens {
            self.max_context_tokens = est;
        }
    }

    /// 현재 문자 추정 컨텍스트 토큰(폴백/캘리브레이션 기준).
    fn context_estimate(&self) -> u64 {
        (self.cumulative_chars_sent / CHARS_PER_TOKEN_EST).max(self.max_context_tokens)
    }

    /// 트랜스크립트를 (있으면) 재파싱해 최신 실 컨텍스트 footprint 를 갱신·반환한다(best-effort). 파일
    /// 부재/파싱 실패면 기존 값 유지. ★탭이 하네스를 실패시키지 않는다★(ADR-0008 경계).
    ///
    /// ★lazy 재검색★: transcript_path 가 아직 None 이면(초기 locate 실패 — claude 가 첫 턴 처리 후에야
    ///   파일을 쓰기 때문, 스모크 실측) session_id 로 한 번 더 검색한다. 찾으면 path 를 캐시한다.
    fn refresh_real_context(&mut self) -> Option<u64> {
        if self.transcript_path.is_none() {
            if let Some(sid) = &self.session_id {
                self.transcript_path = transcript::locate_transcript(sid);
            }
        }
        let path = self.transcript_path.as_deref()?;
        if let Some(summary) = transcript::parse_transcript(path) {
            if let Some(last) = summary.real_usage_series.last() {
                self.latest_real_context = Some(last.context_footprint());
            }
        }
        self.latest_real_context
    }

    /// 이 턴의 usage 스냅샷을 조립 — 실측(있으면)과 추정을 둘 다 담는다(캘리브레이션).
    fn usage_snapshot(&self, decoded: Option<(u64, u64)>) -> UsageSnapshot {
        let (input, output) = decoded.unwrap_or((0, 0));
        UsageSnapshot {
            input_tokens: input,
            output_tokens: output,
            context_tokens_real: self.latest_real_context,
            context_tokens_estimate: self.context_estimate(),
        }
    }

    /// ★finding 2★: 추정-only 감지 샘플 1개를 estimate_samples 에 밀어 넣는다(모든 턴 공통). 실측은 절대
    ///   여기 섞지 않는다 — 감지 계열은 단일 소스라야 소스 전환 인공 급감이 없다(위 estimate_samples 주석).
    fn push_estimate_sample(&mut self, harness_reset: bool) {
        self.estimate_samples.push(UsageSample {
            turn_idx: self.turn_idx,
            context_tokens: self.context_estimate(),
            harness_reset,
        });
    }
}

struct PendingProbe {
    k: u32,
    remaining_gap: u32,
    sender_name: String,
    msg_id: String,
    codeword: String,
}

/// 한 유저 턴을 보내고 그 턴의 종료(MessageDone)를 기다린다. usage/turn 레코드를 쓴다.
#[allow(clippy::too_many_arguments)]
fn drive_turn(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
    kind: &str,
    doc_n: u32,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> TurnResult {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    // 유저 턴 전송 = write_stdin(세션이 wrap_user_turn 으로 감쌈).
    if manager.write_stdin(agent_id, prompt.as_bytes()).is_err() {
        return TurnResult::Terminal;
    }

    // 턴 종료 대기(TURN_WAIT_CAP). 그 사이 에이전트가 죽었는지 목록으로도 확인.
    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let wallclock_ms = t0.elapsed().as_millis() as u64;

    if !ended {
        // 에이전트가 죽어서 종료 못 온 건지, 순수 타임아웃인지 구분.
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                "turn wait cap exceeded".to_string()
            } else {
                "agent terminated before turn end".to_string()
            },
            waited_ms: wallclock_ms,
        }));
        return if alive {
            TurnResult::Stalled
        } else {
            TurnResult::Terminal
        };
    }

    // ★fill 진행은 보낸 문자수 기반 추정(usage 아님 — RunState.cumulative_chars_sent 주석)★.
    state.account_context(prompt.len());
    // 트랜스크립트 탭(있으면) 재파싱으로 이 턴의 최신 실 컨텍스트 footprint 갱신(best-effort).
    state.refresh_real_context();

    // usage 스냅샷 — 실측(트랜스크립트 있으면)과 추정을 둘 다 기록(캘리브레이션). 항상 1건 기록해
    //   실측만 있고 디코딩 usage 는 없는 턴도 계열에 남긴다.
    let usage = Some(state.usage_snapshot(obs.last_usage()));
    // ★finding 2★: 감지 계열은 추정-only(소스 혼합 없음). harness_reset 은 이제 항상 false — 강제 /compact
    //   phase 를 제거해(finding 3) 하네스가 의도적으로 리셋하는 턴 개념 자체가 없어졌다.
    state.push_estimate_sample(false);

    writer.write(&Record::Turn(TurnRecord {
        idx: state.turn_idx,
        kind: kind.to_string(),
        chars_sent: prompt.len(),
        body_sha256: sha256_hex(prompt.as_bytes()),
        usage,
        wallclock_ms,
    }));
    let _ = doc_n; // doc 번호는 sha 로 이미 대조 가능 — 레코드에 별도 미기록(원문 미기록 불변식).
    state.turn_idx += 1;

    // ★finding 3★: MessageDone 으로 끝났어도 그 사이 비-compaction API 에러가 관측됐으면 실패다
    //   (turn_idx 는 이미 올렸으니 이 턴은 소비된 것으로 셈 — 캡·계열 일관). abort 로 상위에 알린다.
    if let Some(e) = obs.turn_error() {
        return TurnResult::Error(e);
    }
    TurnResult::Ok
}

/// 주입 실행 — 실 control 경로(handle_send)로 inter-agent 메시지를 배달하고 레코드를 쓴다.
struct InjectionMeta {
    sender_name: String,
    msg_id: String,
    codeword: String,
}

/// do_injection 결과 — 성공(메타) 또는 abort(사유). ★finding 1★: 주입도 첫급 턴이라 스톨/실패면 런을
/// 중단해야 한다(부분 상태로 다음 프로브가 오귀속되지 않게).
enum InjectOutcome {
    Ok(InjectionMeta),
    Abort(String),
}

/// ★finding 1 fix — 주입을 첫급 fenced 턴으로★: 주입은 handle_send → write_stdin_observed 로 **완결된
///   stream-json user 턴**을 에이전트에 넣는다. 이전엔 이 턴을 펜싱하지 않아(begin_turn/wait_turn_end
///   없음, turn_idx 미증가) 주입의 MessageDone 이 다음 fill/probe 의 wait_turn_end 를 조기 해제 →
///   응답이 엉뚱한 턴에 귀속됐다. 이제: begin_turn → done 스냅샷 → handle_send(=stdin write) →
///   wait_turn_end(baseline) → TurnRecord(kind=inject) → turn_idx++ 로 다른 턴과 동일하게 펜싱한다.
#[allow(clippy::too_many_arguments)]
fn do_injection(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    obs: &Arc<TurnObserver>,
    agent_id: AgentId,
    agent_epoch: u32,
    run_id: &str,
    k: u32,
    at_fraction: f64,
    seed: u64,
    delivery_seen: &Arc<Mutex<Vec<DeliveryObservation>>>,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> InjectOutcome {
    // 발신자 신원 발급(control_send.rs 패턴 — 등록된 발신자 토큰).
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, format!("pilot-sender-{k}-{run_id}"));
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    let sender_name = sender.to_string()[..8].to_string(); // profile 부재 → id 앞8자 fallback.

    // run-scoped 마커 + 결정적 codeword(seed 파생).
    //   ★프레이밍(스모크 실측 2026-07-20)★: 명령형 "report ..." 문구는 인젝션으로 오인돼 거부됐다 —
    //   그래서 회상 대상(codeword)을 담되 **친근한 메모**로 프레이밍한다(명령 없음). 회상은 나중 프로브
    //   질문이 요청하므로, 이 메시지 본문은 codeword 를 자연스럽게 담기만 하면 된다.
    let codeword = codeword_for(seed, k);
    let body = format!(
        "Just a friendly note to keep in mind for later: our shared codeword for this session is {codeword} (reference {run_id}-{k}). No action needed right now — thanks!"
    );
    // ★finding 6★: TurnRecord 의 sha256·chars_sent 를 **이 authored body** 로 정합시킨다(둘 다 같은 대상).
    //   봉투(wrap_message)로 감싼 실 전송 바이트수는 InjectionRecord.bytes_requested 가 따로 담는다.
    let body_len = body.len();
    let body_sha256 = sha256_hex(body.as_bytes());

    // 수신자 지목 = 정확한 AgentId 문자열(profile name 대신 id — 스폰 name 과 무관하게 견고).
    let to = agent_id.to_string();

    // ★펜싱 시작(finding 1)★: 주입 stdin write 직전에 턴 리셋 + done 스냅샷.
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    let before = delivery_seen.lock().unwrap().len();
    let result = handle_send(
        manager,
        registry,
        Entrance::Cli,
        ControlCommand { from, to, body },
    );
    let v = result.to_json();
    let msg_id = v["id"].as_str().unwrap_or("").to_string();

    // 배달 관측(성공 시 새 레코드 1건).
    let delivery = {
        let g = delivery_seen.lock().unwrap();
        g.get(before).cloned()
    };
    let (delivered, bytes_requested, bytes_written, to_epoch, error) = match &delivery {
        Some(o) => (
            o.is_delivered(),
            o.bytes_requested,
            o.bytes_written,
            o.to_epoch,
            o.error.clone(),
        ),
        None => (v["status"] == "enqueued", 0, None, None, None),
    };

    writer.write(&Record::Injection(InjectionRecord {
        k,
        at_fraction,
        msg_id: msg_id.clone(),
        codeword: codeword.clone(),
        sender_name: sender_name.clone(),
        delivered,
        bytes_requested,
        bytes_written,
        to_epoch,
        error: error.clone(),
    }));
    let _ = agent_epoch; // epoch 핀 없음(ADR-0086 F5/ADR-0089) — to_epoch 은 관측만.

    // ★배달 실패면 stdin 에 실제 user 턴이 안 들어갔다 — wait_turn_end 를 기다리면 헛되이 타임아웃한다.
    //   그래서 배달 실패 시엔 펜싱을 건너뛰고 turn_idx 만 올린 뒤(주입 시도 = 소비된 턴) abort 시그널.
    if !delivered {
        // ★turn-index 연속성 계약(finding 1)★: turn_idx 를 소비하는 **모든** 경로는 TurnRecord 를 남긴다 —
        //   실패 경로에서도. 안 그러면 전역 turn 인덱스 수열에 구멍이 나 소비 인덱스↔레코드 매핑이 깨진다.
        //   실 벽시계(t0.elapsed)를 싣고 usage 는 못 잡았으니 None(조작된 0 금지).
        writer.write(&Record::Turn(TurnRecord {
            idx: state.turn_idx,
            kind: "inject".to_string(),
            chars_sent: body_len,
            body_sha256: body_sha256.clone(),
            usage: None,
            wallclock_ms: t0.elapsed().as_millis() as u64,
        }));
        state.turn_idx += 1;
        return InjectOutcome::Abort(format!(
            "injection k={k} not delivered: {}",
            error.unwrap_or_else(|| v["code"].as_str().unwrap_or("unknown").to_string())
        ));
    }

    // ★펜싱 완료(finding 1)★: 주입의 자기 MessageDone 을 기다린다 — 그래야 다음 턴이 주입의 done 을
    //   자기 것으로 오인하지 않는다. 그 뒤 TurnRecord(kind=inject) + turn_idx++.
    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let wallclock_ms = t0.elapsed().as_millis() as u64;
    if !ended {
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                "injection turn wait cap exceeded".to_string()
            } else {
                "agent terminated during injection turn".to_string()
            },
            waited_ms: wallclock_ms,
        }));
        // ★turn-index 연속성 계약(finding 1)★: 스톨 경로도 turn_idx 를 소비하므로 TurnRecord 를 남긴다.
        //   실 대기 시간(wallclock_ms)을 싣고 usage 는 못 잡았으니 None. StallRecord 는 별도 진단 레코드고,
        //   이 TurnRecord 는 소비 인덱스↔레코드 매핑을 채우는 목적(둘은 상보적).
        writer.write(&Record::Turn(TurnRecord {
            idx: state.turn_idx,
            kind: "inject".to_string(),
            chars_sent: body_len,
            body_sha256: body_sha256.clone(),
            usage: None,
            wallclock_ms,
        }));
        state.turn_idx += 1;
        return InjectOutcome::Abort(format!("injection k={k} turn did not complete"));
    }

    // 주입 턴의 컨텍스트·usage 계열 반영(다른 턴과 동일 규율). 문자 수는 본문 길이로 근사.
    state.account_context(bytes_requested);
    state.refresh_real_context();
    state.push_estimate_sample(false); // finding 2: 추정-only 감지 계열.

    // ★finding 6 fix — 해시·길이 정합★: inject TurnRecord 는 이제 **우리가 작성한 note body**(codeword 를
    //   담은 실 본문)의 sha256 과 그 body 길이를 함께 실어 hash↔len 이 같은 대상을 가리킨다. 이전엔
    //   chars_sent 로 래핑된 봉투 길이(bytes_requested)를 쓰면서 body_sha256 는 codeword 만 해시해 서로 다른
    //   대상을 가리켰다(계약 위반). 전송 봉투(wrap_message)의 실 바이트수는 InjectionRecord.bytes_requested
    //   가 별도로 담으므로(관측 레코드), TurnRecord 는 authored body 로 일관되게 맞춘다.
    writer.write(&Record::Turn(TurnRecord {
        idx: state.turn_idx,
        kind: "inject".to_string(),
        chars_sent: body_len,
        body_sha256: body_sha256.clone(), // authored note body(codeword 포함)의 sha256 — chars_sent 와 정합.
        usage: Some(state.usage_snapshot(obs.last_usage())),
        wallclock_ms,
    }));
    state.turn_idx += 1;

    // 주입 턴 중 비-compaction 에러도 실패로 승격(finding 3 일관).
    if let Some(e) = obs.turn_error() {
        return InjectOutcome::Abort(format!("injection k={k} turn error: {e}"));
    }

    InjectOutcome::Ok(InjectionMeta {
        sender_name,
        msg_id,
        codeword,
    })
}

/// 지연 프로브 실행 — 회상 질문을 보내고 응답을 채점해 레코드를 쓴다. ★finding 2★: 스톨/실패면 Err(abort).
fn run_probe(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    probe: &PendingProbe,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> Result<(), String> {
    // ★패턴 브레이크(round-3 fix, ADR-0090)★: 이전 라운드에선 프로브가 fill 패턴에 갇혀 `received N` 만
    //   돌려줬다(회상이 아니라 패턴락을 측정 — 판정 도구 무효). 그래서 프로브 문구가 **명시적으로**
    //   "이번 한 답만 문서 연습을 멈춰라 · `received N` 으로 답하지 마라" 고 패턴을 끊고, 완결 문장으로
    //   발신자/메시지 id/codeword 를 진술하게 한다. 명령형은 인젝션 거부를 유발하므로(스모크 실측
    //   claude-sonnet-4-6) 협조적 어조를 유지한다(이건 사용자가 요청한 기억력 연습의 일부).
    let question =
        "Let's PAUSE the document exercise for just this one reply — please do NOT answer \
         `received N` this time. Earlier in this same session, a friendly note from another \
         participant was passed along to you, and it carried a shared codeword. In one or two \
         complete sentences, could you tell me: (a) who the note was from (their name or short id), \
         (b) the reference id that came with it, and (c) the exact codeword it contained? \
         Afterwards we'll resume the document exercise as normal. Thanks!"
            .to_string();
    let out = send_and_collect(manager, agent_id, obs, &question, state, "probe", writer);

    // ★finding 4★: 프로브도 실 턴이므로 TurnRecord(kind="probe")를 실측 turn_idx·wallclock·usage 로 남긴다
    //   (인덱스 구멍 방지). body 는 프로브 질문(원과제 필러가 아니라 실험 메타라 sha256+len 기록 OK).
    writer.write(&Record::Turn(TurnRecord {
        idx: out.turn_idx,
        kind: "probe".to_string(),
        chars_sent: question.len(),
        body_sha256: sha256_hex(question.as_bytes()),
        usage: out.usage,
        wallclock_ms: out.wallclock_ms,
    }));

    let scores = score_probe(
        &out.response,
        &probe.sender_name,
        &probe.msg_id,
        &probe.codeword,
        false,
        0,
        "",
    );
    writer.write(&Record::Probe(ProbeRecord {
        for_injection_k: Some(probe.k),
        turn_idx: out.turn_idx, // finding 4: 프로브가 소비한 실 턴 인덱스(TurnRecord 와 정렬).
        usage: out.usage,
        question,
        response: cap_response(&out.response),
        final_report: false,
        scores,
    }));
    // ★finding 2★: 스톨/실패면 abort — 늦은 result 가 다음 wait 를 오귀속하지 못하게 런을 여기서 끊는다.
    match out.abort {
        Some(r) => Err(r),
        None => Ok(()),
    }
}

/// FINAL REPORT 프로브 — 원과제 완료(문서 수 + DOC-1 제목). ★finding 2★: 스톨/실패면 Err(abort).
#[allow(clippy::too_many_arguments)]
fn run_final_report(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    _probe: &PendingProbe,
    cfg: &PilotConfig,
    doc_count: u32,
    state: &mut RunState,
    writer: &mut JsonlWriter,
) -> Result<(), String> {
    // ★패턴 브레이크(round-3 fix, ADR-0090)★: FINAL REPORT 도 fill 패턴을 명시적으로 멈추고 완결 문장으로
    //   답하게 한다(이전 라운드에선 빈 답/`received N` 로 회귀). 협조적 어조 유지(인젝션 거부 회피).
    let question =
        "That's the end of the document exercise — thank you! Now please PAUSE the `received N` \
         pattern for this final answer. In one full sentence, could you give me the FINAL REPORT: \
         the total number of documents you received during this session, and the exact title of \
         DOC-1 (the first document)?"
            .to_string();
    let out = send_and_collect(manager, agent_id, obs, &question, state, "final", writer);

    // ★finding 4★: FINAL REPORT 도 실 턴 — TurnRecord(kind="final")를 실측 인덱스·wallclock·usage 로 남긴다.
    writer.write(&Record::Turn(TurnRecord {
        idx: out.turn_idx,
        kind: "final".to_string(),
        chars_sent: question.len(),
        body_sha256: sha256_hex(question.as_bytes()),
        usage: out.usage,
        wallclock_ms: out.wallclock_ms,
    }));

    let expected_title = doc_title(cfg.seed, 1);
    let scores = score_probe(&out.response, "", "", "", true, doc_count, &expected_title);
    writer.write(&Record::Probe(ProbeRecord {
        for_injection_k: None,
        turn_idx: out.turn_idx, // finding 4: FINAL 이 소비한 실 턴 인덱스(TurnRecord 와 정렬).
        usage: out.usage,
        question,
        response: cap_response(&out.response),
        final_report: true,
        scores,
    }));
    match out.abort {
        Some(r) => Err(r),
        None => Ok(()),
    }
}

/// send_and_collect 결과 — 응답 텍스트 + 진행 가능 여부. ★finding 2★: 이전엔 String 만 돌려줘 타임아웃·
/// write 실패가 부분 텍스트로 조용히 완료됐다(turn_idx 미증가·stall 미기록·abort 미게이트). 이제
/// drive_turn 과 대칭으로: timeout/write-fail 시 Stall 기록 + turn_idx++ + abort 시그널(should_abort).
struct CollectOutcome {
    response: String,
    /// Some 이면 호출자는 이 사유로 런을 abort 해야 한다(늦은 result 가 다음 wait 를 조용히 완료 못 하게).
    abort: Option<String>,
    /// ★finding 4★: 이 턴이 소비한 turn_idx(호출자가 프로브/final TurnRecord·ProbeRecord 에 실측 인덱스로
    ///   싣는다 — 인덱스 구멍 방지). send_and_collect 가 turn_idx 를 올리기 **전** 값 = 이 턴의 인덱스.
    turn_idx: u32,
    /// ★finding 4★: 이 턴의 usage 스냅샷(실측 + 추정 — TurnRecord/ProbeRecord 공용). 스톨/write-fail 시엔
    ///   None(usage 를 못 잡은 턴).
    usage: Option<UsageSnapshot>,
    /// ★finding 4★: 이 턴의 실 벽시계 ms(TurnRecord 용 — 조작된 0 금지). 스톨/write-fail 시엔 대기한 ms.
    wallclock_ms: u64,
}

/// 유저 턴 전송 + 턴 종료 대기 + 응답 텍스트 회수(공통). turn 레코드는 쓰지 않고(호출자가 probe/compact
/// 레코드로 감쌈) usage 샘플/turn_idx 는 갱신한다. ★finding 2★: drive_turn 과 대칭 — 실패 시 Stall 기록 +
/// turn_idx++ + abort 시그널. **어떤 경우에도 wait_turn_end 로 이 턴을 펜싱한 뒤 반환**(다음 write 전에).
fn send_and_collect(
    manager: &Arc<AgentManager>,
    agent_id: AgentId,
    obs: &Arc<TurnObserver>,
    prompt: &str,
    state: &mut RunState,
    kind: &str,
    writer: &mut JsonlWriter,
) -> CollectOutcome {
    obs.begin_turn();
    let baseline = obs.done_snapshot();
    let t0 = Instant::now();

    // write 실패 = 에이전트 죽음/전송 불가 — Stall 기록 + turn_idx++ + abort(대칭).
    if manager.write_stdin(agent_id, prompt.as_bytes()).is_err() {
        // ★finding 2 fix — 조작된 0 금지★: t0 는 write 시도 **전**에 잡혔으므로 이 경로에서도 실 경과가
        //   측정 가능하다. write_stdin 이 블로킹/재시도하다 실패하면 0 이 아닌 실제 소요를 남긴다.
        let elapsed_ms = t0.elapsed().as_millis() as u64;
        let this_idx = state.turn_idx; // finding 4: 이 턴이 소비한 인덱스(올리기 전).
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: "write_stdin failed (agent gone)".to_string(),
            waited_ms: elapsed_ms,
        }));
        state.turn_idx += 1;
        return CollectOutcome {
            response: String::new(),
            abort: Some(format!("{kind} turn write failed")),
            turn_idx: this_idx,
            usage: None,
            wallclock_ms: elapsed_ms,
        };
    }

    let ended = obs.wait_turn_end(baseline, TURN_WAIT_CAP);
    let waited_ms = t0.elapsed().as_millis() as u64;
    if !ended {
        // 타임아웃/죽음 — Stall 기록 + turn_idx++ + abort. ★핵심(finding 2)★: turn_idx 를 올려야 늦게
        //   도착한 result(MessageDone)가 다음 턴의 wait_turn_end 를 조용히 완료하지 못한다(오귀속 차단).
        let this_idx = state.turn_idx; // finding 4: 이 턴이 소비한 인덱스(올리기 전).
        let alive = manager.list_agents().iter().any(|a| a.id == agent_id);
        writer.write(&Record::Stall(StallRecord {
            turn_idx: state.turn_idx,
            reason: if alive {
                format!("{kind} turn wait cap exceeded")
            } else {
                format!("{kind} agent terminated before turn end")
            },
            waited_ms,
        }));
        let response = obs.response_text(); // 부분 응답도 담아 반환(진단용).
        state.turn_idx += 1;
        return CollectOutcome {
            response,
            abort: Some(format!("{kind} turn stalled")),
            turn_idx: this_idx,
            usage: None,
            wallclock_ms: waited_ms,
        };
    }

    // fill 진행은 문자 추정(usage 아님 — drive_turn 과 동일 규율) + 트랜스크립트 재파싱으로 실측 갱신.
    state.account_context(prompt.len());
    state.refresh_real_context();
    state.push_estimate_sample(false); // finding 2: 추정-only 감지 계열(harness_reset 개념 폐기 — finding 3).
    let this_idx = state.turn_idx; // finding 4: 이 턴이 소비한 인덱스(올리기 전).
    let usage = Some(state.usage_snapshot(obs.last_usage()));
    state.turn_idx += 1;
    // 비-compaction API 에러도 abort 로(finding 3 일관).
    let abort = obs.turn_error().map(|e| format!("{kind} turn error: {e}"));
    CollectOutcome {
        response: obs.response_text(),
        abort,
        turn_idx: this_idx,
        usage,
        wallclock_ms: waited_ms,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════
// cleanup / 유틸
// ═══════════════════════════════════════════════════════════════════════════════════

/// cleanup 이 제거할 per-run 임시 경로 묶음. ★finding 9★: data_dir·workspace 만이 아니라 profile/preset
/// temp 도 함께 제거한다(이전엔 profile/preset 이 누수됐다).
struct CleanupPaths<'a> {
    data_dir: &'a std::path::Path,
    workspace: &'a std::path::Path,
    profile_dir: &'a std::path::Path,
    preset_dir: &'a std::path::Path,
}

async fn cleanup(
    manager: &Arc<AgentManager>,
    agent_id: Option<AgentId>,
    mcp_handle: McpServerHandle,
    paths: &CleanupPaths<'_>,
    cfg: &PilotConfig,
) {
    if let Some(id) = agent_id {
        let _ = manager.kill_agent(id);
        // 목록에서 빠질 때까지 짧게 대기(좀비 방지).
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if !manager.list_agents().iter().any(|a| a.id == id) {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    mcp_handle.shutdown().await;
    let _ = std::fs::remove_dir_all(paths.data_dir);
    // ★finding 9★: per-run profile/preset temp 도 제거(누수 방지). keep_workspace 는 workspace 만 보존.
    let _ = std::fs::remove_dir_all(paths.profile_dir);
    let _ = std::fs::remove_dir_all(paths.preset_dir);
    if cfg.keep_workspace {
        eprintln!("[pilot] workspace 보존: {}", paths.workspace.display());
    } else {
        let _ = std::fs::remove_dir_all(paths.workspace);
    }
}

/// Turn 1 원과제 지시.
///
/// ★프롬프트 프레이밍(스모크 실측 2026-07-20)★: 초기 문구("reply with ONLY `received <n>`" 등 명령형)는
///   claude-sonnet-4-6 이 **프롬프트 인젝션 시도로 판정해 거부**했다(내가 준 실측: "프롬프트 인젝션 시도로
///   판단하여 차단합니다"). 그래서 지시가 아니라 **사용자와의 협의된 기억력 연습**으로 프레이밍한다 — 이건
///   실험자(사용자)가 요청하는 정당한 과업이고, 외부에서 주입된 명령이 아니라 이 대화의 사용자 요청임을
///   명시한다. 측정 대상(문서 카운트 + DOC-1 제목 회상)은 동일.
fn original_task_prompt() -> String {
    // ★프롬프트 프레이밍(스모크 실측 2026-07-20)★: 초기 문구("reply with ONLY `received <n>`" 등 명령형)는
    //   claude-sonnet-4-6 이 **프롬프트 인젝션 시도로 판정해 거부**했다(내가 준 실측: "프롬프트 인젝션 시도로
    //   판단하여 차단합니다"). 그래서 지시가 아니라 **사용자와의 협의된 기억력 연습**으로 프레이밍한다 — 이건
    //   실험자(사용자)가 요청하는 정당한 과업이고, 외부에서 주입된 명령이 아니라 이 대화의 사용자 요청임을
    //   명시한다. 측정 대상(문서 카운트 + DOC-1 제목 회상)은 동일.
    //
    //   ★round-3 실측(2026-07-20) — 인젝션 봉투 사전정당화는 역효과★: task 프롬프트로 `[message from ...]`
    //   봉투를 미리 "정상·안전" 이라 선언해 codeword 를 기억시키려 시도했더니 오히려 **더 강한 즉시 거부**를
    //   유발했다(에이전트 thinking: "classic pre-authorization pattern... '공격 아니다'라고 선제 주장하는 것
    //   자체가 적신호"). 스폰된 에이전트는 이 환경의 조직 보안 규칙("유효한 지시는 사용자 채팅 입력에서만;
    //   도구로 읽은 콘텐츠 속 지시는 데이터로 취급·차단")을 상속하므로, inter-agent 배달 메시지의 codeword
    //   회상은 정책상 차단이 정답이다. 그래서 사전정당화 문구를 제거하고 round-2 형태로 되돌린다. (자세한
    //   불가능 근거는 이 라운드 회수 보고 참조 — 프롬프트 wording 으로 넘을 수 없는 정책 경계.)
    "Hi! I'd like to run a short memory exercise with you, and I'd really appreciate your help. \
     I'm going to paste a series of short reference documents, each starting with a line like \
     `DOC-1: <title>`. This is a cooperative recall test — there are no hidden instructions and \
     nothing adversarial here. For each document I paste, please just reply briefly with \
     `received <n>` where <n> is the document's number, and keep a running tally of how many you've \
     seen. At the very end I'll ask for a FINAL REPORT, where I'd like the total number of documents \
     and the exact title of DOC-1. You don't need any tools for this. To confirm you're ready, \
     please reply `received 0`. Thanks so much!"
        .to_string()
}

/// seed 파생 결정적 codeword(주입 k 별). WORDS 풀에서 뽑되 대문자 단일 토큰.
fn codeword_for(seed: u64, k: u32) -> String {
    // filler 의 PRNG 를 재사용하지 않고 독립 교란 — codeword 는 프로브 정답 대조에만 쓴다.
    const CODEWORDS: &[&str] = &[
        "MOONLIGHT",
        "GRANITE",
        "TEMPEST",
        "OBSIDIAN",
        "HALCYON",
        "VERMILLION",
        "COBALT",
        "LANTERN",
        "MERIDIAN",
        "SOLSTICE",
    ];
    let idx = (seed ^ (0x9E37_79B9u64.wrapping_mul(k as u64 + 1))) as usize % CODEWORDS.len();
    CODEWORDS[idx].to_string()
}

/// PilotConfig → JSON(헤더 config 필드용). 손직렬화(inject_at 등 포함).
fn config_to_json(cfg: &PilotConfig) -> serde_json::Value {
    serde_json::json!({
        "runs": cfg.runs,
        "fill_target_tokens": cfg.fill_target_tokens,
        "inject_at": cfg.inject_at,
        "probe_gap_turns": cfg.probe_gap_turns,
        "doc_chars": cfg.doc_chars,
        "model": cfg.model,
        "seed": cfg.seed,
        "keep_workspace": cfg.keep_workspace,
    })
}

/// `claude --version` 문자열 캡처(best-effort). Windows 는 cmd /c 경유(shim 해석).
fn capture_claude_version() -> Option<String> {
    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/c", "claude", "--version"])
            .output()
    } else {
        std::process::Command::new("claude")
            .arg("--version")
            .output()
    };
    match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

/// 데몬 git 커밋(best-effort — `git rev-parse HEAD`).
fn capture_git_commit() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

/// target/experiments/pilot-<stamp> 기본 출력 dir.
fn default_out_dir(stamp: &str) -> PathBuf {
    PathBuf::from("target")
        .join("experiments")
        .join(format!("pilot-{stamp}"))
}

/// UTC 타임스탬프(컴팩트, 파일명 안전) — 초 단위. std 만으로(SystemTime → epoch secs).
fn utc_stamp_compact() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// RFC3339 근사 UTC 문자열(초 정밀) — 신규 의존성 없이 epoch 초를 그대로 노출한다(정확 캘린더 변환은
/// chrono 등 필요 → no-new-deps 제약상 epoch-secs + Z 표기로 대체). 사후 분석이 초를 캘린더로 변환한다.
fn utc_stamp_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("epoch:{secs}Z")
}

/// JSONL 파일 라이터 — 레코드마다 한 줄 + flush. flush 로 abort 시에도 부분 파일이 온전하게 남는다.
///
/// ★finding 10 fix★: 파일을 **truncate(create+write)** 로 연다(append 아님). 이전엔 append 라 같은
///   --out 을 재사용하면 두 런의 레코드가 run-0.jsonl 안에 뒤섞였다(파싱 시 런 경계 붕괴). truncate 로
///   열면 각 런 파일은 항상 그 런만 담는다. 헤더-first 계약은 호출자(run body)가 HeaderRecord 를 가장
///   먼저 write 해 지킨다.
/// ★finding 8 fix★: write/flush 에러를 무시하지 않고 누적 카운트한다 — summary 직전 이 카운트를 보고
///   기록 손실 여부를 진단할 수 있다(디스크 풀 등 조용한 실패 가시화).
struct JsonlWriter {
    file: std::fs::File,
    /// write/flush 실패 누적(finding 8 — 조용한 기록 손실 가시화).
    io_errors: u64,
}
impl JsonlWriter {
    fn create(path: &std::path::Path) -> std::io::Result<Self> {
        // truncate: create + write + truncate(append 금지 — 런 혼합 방지). 기존 파일은 덮어쓴다.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self { file, io_errors: 0 })
    }
    /// 레코드 1개를 한 줄로 write + flush(중간 저장 — abort 안전). 실패는 io_errors 로 누적.
    fn write(&mut self, rec: &Record) {
        let line = rec.to_jsonl_line();
        self.write_raw_line(&line);
    }

    /// 임의의 원시 한 줄을 write + flush. 헤더 이전 setup abort 마커(wiring_failed) 전용 — 정식 레코드가
    /// 아니라 스키마 밖 진단 라인이므로 Record 를 거치지 않는다(accepted residual).
    fn write_raw_line(&mut self, line: &str) {
        if writeln!(self.file, "{line}").is_err() {
            self.io_errors += 1;
            return;
        }
        if self.file.flush().is_err() {
            self.io_errors += 1;
        }
    }
}
