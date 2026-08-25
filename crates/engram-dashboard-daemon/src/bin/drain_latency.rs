//! ★ADR-0125 동기 드레인 지연 실측 드라이버(spec §7 수용 기준 ⑤·⑧)★ — 발송 호출이 **자기 턴에
//! 떠안는** 드레인 비용을 재서 숫자만 낸다.
//!
//! ★타이밍 단언이 없다(spec §7 ⑤·⑧ 이 명시적으로 금지)★: 임계값·목표치를 여기 박으면 ADR-0125 가
//!   열어 둔 질문(수신자별 동시성 미도입)에 하네스가 몰래 답하는 것이 된다. 그래서 이 bin 이 실패로
//!   끝나는 경우는 **측정 전제가 무너졌을 때뿐**이다 — 보관함 cap 경계가 옮겨졌거나(아래 `MAILBOX_CAP`)
//!   프리필이 요청한 파킹 수를 못 만들었을 때.
//! ★회귀 스위트에 넣지 않는다★: 시간 측정은 머신 부하에 흔들려 게이트로 쓰면 flaky 하다.
//!
//! ★`obs_seam` 최소 복제(정직 표기)★: 결정적 수신자(구조화 캐리어를 흉내 내면서 write 를 캡처하는
//!   transport)는 `tests/control_send.rs` 의 `obs_seam` 과 같은 기법인데, integration test 의 모듈은
//!   bin 에서 import 할 수 없다. 그래서 **이 측정에 필요한 최소분**(transport·caps·세션 삽입·이름 파생)
//!   만 여기 다시 적었다 — 그쪽의 실패 주입·terminal 세션·동명 주입·봉투 재구성은 가져오지 않았다.
// ADR-0125

use std::path::PathBuf;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::backend::InputEncoder;
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::ProfileRegistry;
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps,
    OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
};

use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, ControlResult};
use engram_dashboard_daemon::control::registry::{BoundIdentity, ControlRegistry};
use engram_dashboard_daemon::messaging_host::messaging_for_manager_gated;
use engram_dashboard_messaging::busy::{BusyPolicy, IdleNotifier, ScriptedTurnFacts};
use engram_dashboard_messaging::envelope::Entrance;
use engram_dashboard_messaging::service::{FlushTrigger, MessagingService};
use engram_dashboard_messaging::PeerId;

/// ★정본은 `engram-dashboard-messaging` 의 `mailbox.rs` 상수 `MAILBOX_CAP` 이고 그건 비공개라 여기 값을
/// 복제했다★.
const MAILBOX_CAP: usize = 100;

/// ★홀수★ — median 이 두 표본의 평균이 아니라 실제 표본 하나가 된다.
const REPS: usize = 25;

/// 첫 반복은 할당자·페이지 폴트·분기 예측이 차가워 다른 반복보다 몇 배 느리게 나오는데, 그게 max 칸을
/// 통째로 차지하면 표본 산포를 못 읽는다. 버림은 **중앙값이 아니라 min/max 를 읽을 수 있게** 하려는
/// 것이다(중앙값은 어차피 이상치에 둔감하다).
const WARMUP: usize = 3;

/// ⑤ ★실측: 최악은 cap 이 아니라 **cap−1**(=99)이다★ — 거기서 한 호출이 100건을 주입하고, cap(=100)은
/// 반대로 **가장 싼 점**이다(반려가 드레인 **전에** 끊어 주입이 0건이다). 두 점을 함께 재는 이유가
/// 그것이다 — 하나만 재면 어느 쪽이든 라벨이 거짓이 된다.
const BURST_PARKED: &[usize] = &[0, 1, 10, 50, 99, 100];

/// ⑤ ★이게 없으면 "드레인이 4ms 다" 라는 문장만 뽑혀 나간다★ — 범위가 벗겨진 숫자가 그대로 인용된다.
const BURST_CAVEAT: &str =
    "in-process drain cost ONLY · the real stdin write is NOT in the timed region — the harness captures bytes in memory where production does a blocking write_all+flush into the child's pipe/ConPTY, so for rows that inject production is strictly higher and the omitted part is potentially unbounded (up to 64 KiB × a 100-deep batch into one child's pipe, no timeout), while the cap-full row injects nothing and is unaffected";

const FANOUT_N: &[usize] = &[1, 2, 4, 8, 16];

/// ★넓은 이유 = 칼럼 헤더가 범위 태그를 달기 때문이다("in-proc"/"aggregate")★ — 표만 긁어 가면 제목·
/// 꼬리의 고지가 떨어져 나가는데, **헤더 줄은 어떤 표 추출에도 딸려 간다**. 그래서 최소한의 범위 표시를
/// 거기 심고 데이터 칸 너비를 거기 맞춘다(행마다 범위 필드를 다는 것은 표를 부풀리므로 하지 않는다).
const MEDIAN_COL: usize = 19;

/// ⑧ ADR-0125 가 물은 "수신자 A 의 드레인이 B 를 얼마나 늦추나"(수신자별 인과)는 드레인 루프 **안쪽**
/// 타임스탬프가 있어야 해서 여기서 답하지 않는다. 문구를 지우면 다음 세션이 이 숫자를 그 답으로 오독한다.
const FANOUT_CAVEAT: &str =
    "aggregate N-scaling only · per-recipient causality NOT measured (needs production hooks — user decision)";

/// ★실측(리뷰어 2인 + 발주자 + 작성자 = 4회 독립 실행)★: **한 실행 안**에서는 최악 포인트가 ±0.2% 로
/// 안정한데 **실행 사이**엔 고정비가 ~2배, 기울기가 ~30% 움직인다. 그래서 계수를 인용하면 다음 세션이
/// 재현에 실패하고, 재현되는 것은 모양뿐이다.
const VARIANCE_NOTE: &str =
    "debug build · magnitudes vary ~2× across sessions; what reproduces is the shape (linear in batch size, worst ≈ cap−1, cap-full cheapest), not the coefficients";

/// ★훅을 없애려 들지 말 것★: 이 bin 은 `test-harness` 기능을 **켜야만 존재**한다(seam 세션 주입이 같은
/// 기능 뒤에 있다). 남길 것은 제거가 아니라 이 고지다.
const INSTRUMENT_NOTE: &str =
    "test-harness feature is ON (this bin requires it) — the timed path carries hook lookups production never pays (mid-send RwLock read + accept-hook OnceLock lookup); lock-read scale, buried under the cross-session variance above";

// ── 결정적 수신자 seam ──────────────────────────────────────────────────────────────────────────

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// `structured: true` 로 신고해 도달성 게이트를 통과하고, 그 값이 곧 커널의 `LiveAgent::turn_signal` 이
/// 된다 — idle 게이트가 이 수신자에게 성립해야 프리필이 파킹된다.
struct SeamTransport {
    captured: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl AgentTransport for SeamTransport {
    fn start(&self, _core: Arc<OutputCore>) {}
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
        let InputEvent::Raw(bytes) = input;
        self.captured.lock().unwrap().push(bytes);
        Ok(())
    }
    fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
        Ok(())
    }
    fn interrupt(&self) -> Result<(), PtyError> {
        Ok(())
    }
    fn shutdown(&self) {}
    fn capabilities(&self) -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: false,
                structured: true,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }
}

struct Seam {
    id: AgentId,
    name: String,
    written: Arc<Mutex<Vec<Vec<u8>>>>,
}

/// 관측 배선 없는 core 를 쓴다 — 턴 사실은 `ScriptedTurnFacts` 로 직접 심으므로 코어 표를 경유할 이유가
/// 없다.
fn insert_seam(manager: &Arc<AgentManager>) -> Seam {
    let id = AgentId::new_v4();
    let name = id.to_string()[..8].to_string();
    let written = Arc::new(Mutex::new(Vec::new()));
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(NoopStatus),
        TurnWiring::detached(),
    ));
    // 프로필 없는 주입 세션의 canonical name = basename(cwd) 이므로(ADR-0101), cwd 끝을 이름과 맞춰
    // "보이는 이름 = 주소" 를 성립시킨다.
    let session = Arc::new(AgentSession::new(
        id,
        PathBuf::from(format!("seam-root/{name}")),
        0,
        80,
        24,
        Arc::new(AtomicU8::new(0)),
        BackendCaps {
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
        },
        InputEncoder::ClaudeStreamJson,
        true,
        core,
        Box::new(SeamTransport {
            captured: written.clone(),
        }),
    ));
    manager.insert_test_session(session);
    Seam { id, name, written }
}

// ── 배선 ────────────────────────────────────────────────────────────────────────────────────────

/// 운영은 여기서 flush 레인으로 넘겨 다른 스레드가 큐를 비우는데, 이 하네스는 그 레인을 띄우지 않는다 —
/// 그래야 프리필한 파킹이 측정 직전까지 큐에 그대로 남는다. 비용 구조는 운영과 같은 쪽(논블록 enqueue)
/// 이라 타이밍을 왜곡하지 않는다.
struct RecordingBell {
    rings: Mutex<Vec<PeerId>>,
}

impl RecordingBell {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            rings: Mutex::new(Vec::new()),
        })
    }
}

impl IdleNotifier for RecordingBell {
    fn notify_idle(&self, id: PeerId) {
        self.rings.lock().unwrap().push(id);
    }
}

impl FlushTrigger for RecordingBell {
    fn request_flush(&self, id: PeerId) {
        self.rings.lock().unwrap().push(id);
    }
}

struct Stage {
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingService>,
    facts: Arc<ScriptedTurnFacts>,
    store_root: PathBuf,
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.store_root);
    }
}

/// ★반복마다 새로 만든다★ — 하나를 재사용하면 로스터·장부가 반복 수만큼 커져 이름 해석·`@all` 정렬
/// 비용이 뒤 반복에서만 오르고, 그 증가분이 측정치에 섞인다.
fn stage() -> Stage {
    let store_root =
        std::env::temp_dir().join(format!("engram-drain-latency-{}", AgentId::new_v4()));
    let registry = Arc::new(ControlRegistry::new());
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        store_root.join("profiles"),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        store_root.join("presets"),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            sessions_dir: None,
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new(
        Arc::new(NoopStatus),
        profiles,
        presets,
        tracker,
    ));
    let bell = RecordingBell::new();
    let facts = ScriptedTurnFacts::new();
    let busy = Arc::new(BusyPolicy::new(facts.clone(), bell.clone()));
    let messaging = Arc::new(
        messaging_for_manager_gated(manager.clone(), registry.clone(), busy)
            .with_flush_trigger(bell),
    );
    Stage {
        manager,
        registry,
        messaging,
        facts,
        store_root,
    }
}

/// 신원이 registry 에 있어야 발송 경로가 "폐기된 발신자" 경고를 찍지 않는다(경고 자체는 게이트가
/// 아니지만 측정 중 잡음이다).
fn insert_sender(st: &Stage) -> (Seam, BoundIdentity) {
    let seam = insert_seam(&st.manager);
    st.registry
        .issue(seam.id, 0, format!("drain-lat-{}", seam.id), true);
    let from = BoundIdentity {
        agent_id: seam.id,
        epoch: 0,
    };
    (seam, from)
}

fn send(st: &Stage, from: BoundIdentity, to: Vec<String>, body: &str) -> ControlResult {
    let cmd = ControlCommand {
        from,
        to,
        body: body.to_string(),
        contract: Default::default(),
    };
    handle_send(&st.manager, &st.registry, &st.messaging, Entrance::Cli, cmd)
}

fn rows(result: &ControlResult) -> Vec<(&'static str, Option<&'static str>)> {
    match result {
        ControlResult::Ok { results, .. } => results.iter().map(|r| (r.status, r.code)).collect(),
        ControlResult::Error { code, .. } => vec![("reject", Some(code))],
    }
}

fn written_len(seam: &Seam) -> usize {
    seam.written.lock().unwrap().len()
}

// ── 통계 ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Stats {
    median_ns: f64,
    min_ns: u128,
    max_ns: u128,
}

impl Stats {
    fn of(samples: &[u128]) -> Option<Stats> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let mid = sorted.len() / 2;
        let median_ns = if sorted.len() % 2 == 1 {
            sorted[mid] as f64
        } else {
            (sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0
        };
        Some(Stats {
            median_ns,
            min_ns: sorted[0],
            max_ns: *sorted.last().expect("non-empty"),
        })
    }

    /// ★0.1µs 는 노이즈 연극(근거 = `VARIANCE_NOTE`)★ — 소수점을 찍으면 없는 재현성을 주장하게 된다.
    fn us_columns(&self) -> String {
        format!(
            "{:>MEDIAN_COL$.0} {:>10.0} {:>10.0}",
            self.median_ns / 1000.0,
            self.min_ns as f64 / 1000.0,
            self.max_ns as f64 / 1000.0
        )
    }
}

// ── ⑤ 버스트 응답 지연 ──────────────────────────────────────────────────────────────────────────

struct BurstPoint {
    parked: usize,
    injected: usize,
    row: String,
    stats: Stats,
}

fn measure_burst(parked: usize) -> Result<BurstPoint, String> {
    let mut samples = Vec::with_capacity(REPS);
    let mut injected = 0usize;
    let mut row = String::new();

    for rep in 0..(WARMUP + REPS) {
        let st = stage();
        let (_sender, from) = insert_sender(&st);
        let recipient = insert_seam(&st.manager);

        st.facts.set_in_turn(recipient.id, 0, Instant::now());
        for i in 0..parked {
            send(
                &st,
                from,
                vec![recipient.name.clone()],
                &format!("prefill-{i}"),
            );
        }
        let actually_parked = st.messaging.parked_len(&recipient.name);
        if actually_parked != parked {
            return Err(format!(
                "프리필 불일치(rep {rep}): {parked} 건을 요청했는데 큐엔 {actually_parked} 건 — 측정 전제 붕괴"
            ));
        }
        st.facts.set_idle(recipient.id, 0, Instant::now());

        let before = written_len(&recipient);
        let cmd = ControlCommand {
            from,
            to: vec![recipient.name.clone()],
            body: "measured-send".to_string(),
            contract: Default::default(),
        };
        let started = Instant::now();
        let result = handle_send(&st.manager, &st.registry, &st.messaging, Entrance::Cli, cmd);
        let elapsed = started.elapsed();

        if rep >= WARMUP {
            samples.push(elapsed.as_nanos());
        }
        injected = written_len(&recipient) - before;
        row = rows(&result)
            .iter()
            .map(|(s, c)| match c {
                Some(code) => format!("{s}/{code}"),
                None => (*s).to_string(),
            })
            .collect::<Vec<_>>()
            .join(",");
    }

    Ok(BurstPoint {
        parked,
        injected,
        row,
        stats: Stats::of(&samples).expect("REPS > 0"),
    })
}

/// 복제한 상수가 커널과 조용히 갈리면 ⑤의 "최악 케이스" 라벨이 거짓이 되므로 측정을 무효로 만든다.
fn verify_cap_boundary(points: &[BurstPoint]) -> Result<(), String> {
    let at = |n: usize| points.iter().find(|p| p.parked == n);
    if let Some(full) = at(MAILBOX_CAP) {
        if !full.row.contains("MAILBOX_FULL") {
            return Err(format!(
                "파킹 {MAILBOX_CAP} 건에서 반려가 아니라 `{}` — 커널 MAILBOX_CAP 이 {MAILBOX_CAP} 이 아니다",
                full.row
            ));
        }
    }
    if let Some(edge) = at(MAILBOX_CAP - 1) {
        if edge.row.contains("MAILBOX_FULL") {
            return Err(format!(
                "파킹 {} 건에서 이미 반려 — 커널 MAILBOX_CAP 이 {MAILBOX_CAP} 보다 작다",
                MAILBOX_CAP - 1
            ));
        }
    }
    Ok(())
}

// ── ⑧ `@all` fan-out ────────────────────────────────────────────────────────────────────────────

struct FanoutPoint {
    n: usize,
    delivered_rows: usize,
    stats: Stats,
}

/// 턴 사실을 아무것도 심지 않으므로 전원 idle 이다(positive-knowledge-only) — 즉 N명분 드레인이
/// 전부 이 한 호출 안에서 일어난다.
fn measure_fanout(n: usize) -> Result<FanoutPoint, String> {
    let mut samples = Vec::with_capacity(REPS);
    let mut delivered_rows = 0usize;

    for rep in 0..(WARMUP + REPS) {
        let st = stage();
        let (_sender, from) = insert_sender(&st);
        for _ in 0..n {
            insert_seam(&st.manager);
        }

        let cmd = ControlCommand {
            from,
            to: vec!["@all".to_string()],
            body: "fanout-measured".to_string(),
            contract: Default::default(),
        };
        let started = Instant::now();
        let result = handle_send(&st.manager, &st.registry, &st.messaging, Entrance::Cli, cmd);
        let elapsed = started.elapsed();

        let observed = rows(&result);
        delivered_rows = observed.iter().filter(|(s, _)| *s == "delivered").count();
        if delivered_rows != n {
            return Err(format!(
                "@all(N={n}, rep {rep}) 배달 행이 {delivered_rows} — 발신자 제외 후 N명 전원 배달이 전제다"
            ));
        }
        if rep >= WARMUP {
            samples.push(elapsed.as_nanos());
        }
    }

    Ok(FanoutPoint {
        n,
        delivered_rows,
        stats: Stats::of(&samples).expect("REPS > 0"),
    })
}

// ── 보고 ────────────────────────────────────────────────────────────────────────────────────────

fn profile_label() -> &'static str {
    if cfg!(debug_assertions) {
        "debug (unoptimized + debug-assertions)"
    } else {
        "release (optimized)"
    }
}

fn main() {
    println!("=== drain-latency — ADR-0125 동기 드레인 실측 (spec §7 ⑤·⑧) ===");
    println!("측정만 한다 — 임계값 단언 없음(spec §7 이 ⑤·⑧ 양쪽에서 금지)");
    println!();
    println!("  stage        : 결정적 seam 수신자 — 실 claude 없음 · 실 PTY 없음");
    println!("  timed region : engram_dashboard_daemon::control::ingress::handle_send 호출 1회");
    println!("  excluded     : 무대 조립 · 세션 삽입 · 보관함 프리필 · 응답 판독 · 통계 계산");
    println!(
        "               : ★실 stdin 쓰기도 제외★ — seam 이 메모리 캡처로 대신한다(운영은 자식"
    );
    println!("                 파이프/ConPTY 로 blocking write_all+flush). 아래 ⑤ 고지가 정본.");
    println!("  mailbox cap  : {MAILBOX_CAP} (messaging mailbox.rs `MAILBOX_CAP` 복제 — 매 실행 경계 실측 재확인)");
    println!("  reps/point   : {REPS} recorded, {WARMUP} discarded warmup (median / min / max)");
    println!("  build        : {}", profile_label());
    println!(
        "  host         : {} {} · {} logical cpus",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism()
            .map(|p| p.get().to_string())
            .unwrap_or_else(|_| "?".to_string())
    );
    println!("  reproduces   : {VARIANCE_NOTE}");
    println!("  instrument   : {INSTRUMENT_NOTE}");
    println!();

    let mut failed = false;

    println!("⑤ 버스트 응답 지연 — {BURST_CAVEAT}");
    println!(
        "  {:>6} {:>8} {:>22} {:>MEDIAN_COL$} {:>10} {:>10}",
        "parked", "injected", "row", "in-proc median µs", "min µs", "max µs"
    );
    let mut burst_points = Vec::new();
    for &parked in BURST_PARKED {
        match measure_burst(parked) {
            Ok(p) => {
                println!(
                    "  {:>6} {:>8} {:>22} {}",
                    p.parked,
                    p.injected,
                    p.row,
                    p.stats.us_columns()
                );
                burst_points.push(p);
            }
            Err(e) => {
                println!("  {parked:>6} ★측정 무효★ {e}");
                failed = true;
            }
        }
    }
    if let Err(e) = verify_cap_boundary(&burst_points) {
        println!("  ★CAP DRIFT★ {e}");
        failed = true;
    }
    println!();
    println!("⑤ 범위 고지(그대로 옮길 것): {BURST_CAVEAT}");
    println!();

    println!("⑧ @all fan-out — {FANOUT_CAVEAT}");
    println!(
        "  {:>6} {:>18} {:>MEDIAN_COL$} {:>10} {:>10}",
        "N", "delivered rows", "aggregate median µs", "min µs", "max µs"
    );
    for &n in FANOUT_N {
        match measure_fanout(n) {
            Ok(p) => println!(
                "  {:>6} {:>18} {}",
                p.n,
                p.delivered_rows,
                p.stats.us_columns()
            ),
            Err(e) => {
                println!("  {n:>6} ★측정 무효★ {e}");
                failed = true;
            }
        }
    }
    println!();
    println!("⑧ 범위 고지(그대로 옮길 것): {FANOUT_CAVEAT}");

    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 통계 ────────────────────────────────────────────────────────────────

    #[test]
    fn median_of_odd_sample_is_the_middle_value() {
        let s = Stats::of(&[30, 10, 20, 50, 40]).expect("표본 있음");
        assert_eq!(s.median_ns, 30.0);
        assert_eq!(s.min_ns, 10);
        assert_eq!(s.max_ns, 50);
    }

    #[test]
    fn median_of_even_sample_averages_the_middle_pair() {
        let s = Stats::of(&[40, 10, 30, 20]).expect("표본 있음");
        assert_eq!(s.median_ns, 25.0);
        assert_eq!(s.min_ns, 10);
        assert_eq!(s.max_ns, 40);
    }

    #[test]
    fn single_sample_collapses_to_one_value() {
        let s = Stats::of(&[7]).expect("표본 있음");
        assert_eq!((s.median_ns, s.min_ns, s.max_ns), (7.0, 7, 7));
    }

    #[test]
    fn empty_sample_has_no_stats() {
        assert!(Stats::of(&[]).is_none());
    }

    // ── 프리필 ──────────────────────────────────────────────────────────────

    #[test]
    fn prefill_parks_exactly_the_requested_count() {
        let st = stage();
        let (_sender, from) = insert_sender(&st);
        let recipient = insert_seam(&st.manager);
        st.facts.set_in_turn(recipient.id, 0, Instant::now());

        for i in 0..7 {
            send(
                &st,
                from,
                vec![recipient.name.clone()],
                &format!("prefill-{i}"),
            );
        }

        assert_eq!(st.messaging.parked_len(&recipient.name), 7);
        assert_eq!(
            written_len(&recipient),
            0,
            "턴 중이면 주입되지 않는다(파킹만)"
        );
    }

    #[test]
    fn one_send_after_idle_drains_the_whole_backlog() {
        let st = stage();
        let (_sender, from) = insert_sender(&st);
        let recipient = insert_seam(&st.manager);
        st.facts.set_in_turn(recipient.id, 0, Instant::now());
        for i in 0..4 {
            send(
                &st,
                from,
                vec![recipient.name.clone()],
                &format!("prefill-{i}"),
            );
        }
        st.facts.set_idle(recipient.id, 0, Instant::now());

        let result = send(&st, from, vec![recipient.name.clone()], "measured-send");

        assert_eq!(rows(&result), vec![("delivered", None)]);
        assert_eq!(written_len(&recipient), 5, "묵은 4건 + 이번 1건");
        assert_eq!(st.messaging.parked_len(&recipient.name), 0);
    }
}
