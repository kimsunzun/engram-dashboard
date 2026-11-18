//! ★ADR-0125 synchronous drain latency measurement driver (spec §7 acceptance criteria ⑤·⑧)★ — measures the drain cost a send call takes on in its own turn and prints only the numbers.
//!
//! ★No timing assertions (spec §7 ⑤·⑧ forbids them)★: baked-in thresholds or targets would have the harness secretly answer what ADR-0125 left open (per-recipient concurrency not introduced). This bin fails only when a measurement premise collapses — the mailbox cap boundary moved (`MAILBOX_CAP` below) or the prefill could not create the requested number of parkings.
//! ★Not in the regression suite★: time measurements wobble with machine load and are flaky as a gate.
//!
//! ★Minimal copy of `obs_seam`★: the deterministic recipient (a transport mimicking the structured carrier while capturing writes) is the technique of `obs_seam` in `tests/control_send.rs`, but an integration test's modules cannot be imported from a bin. Only the minimum this measurement needs (transport · caps · session insertion · name derivation) is rewritten here; failure injection · terminal sessions · same-name injection · envelope reconstruction were not brought over.
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

/// ★Defined by the private `MAILBOX_CAP` in `mailbox.rs` of `engram-dashboard-messaging`, so the value is copied here★.
const MAILBOX_CAP: usize = 100;

/// ★Odd★ — the median is one real sample, not the mean of two.
const REPS: usize = 25;

/// The first iteration is several times slower (allocator · page faults · branch prediction cold) and would take the whole max cell, making the spread unreadable. Discarding makes min/max readable, not the median (insensitive to outliers anyway).
const WARMUP: usize = 3;

/// ⑤ ★Measured: the worst is not cap but cap−1 (=99)★ — one call injects 100 items there, while cap (=100) is the cheapest point (the bounce cuts before the drain, 0 items injected). Measure both points together — either alone makes the label false.
const BURST_PARKED: &[usize] = &[0, 1, 10, 50, 99, 100];

/// ⑤ ★Without this, only the sentence "the drain is 4ms" gets quoted★ — a number stripped of its scope gets quoted as is.
const BURST_CAVEAT: &str =
    "in-process drain cost ONLY · the real stdin write is NOT in the timed region — the harness captures bytes in memory where production does a blocking write_all+flush into the child's pipe/ConPTY, so for rows that inject production is strictly higher and the omitted part is potentially unbounded (up to 64 KiB × a 100-deep batch into one child's pipe, no timeout), while the cap-full row injects nothing and is unaffected";

const FANOUT_N: &[usize] = &[1, 2, 4, 8, 16];

/// ★Wide because the column header carries the scope tag ("in-proc"/"aggregate")★ — table scraping drops the title and tail notices, but the header line goes along with any table extraction. So the minimal scope marking is planted there (a scope field per row would bloat the table).
const MEDIAN_COL: usize = 19;

/// ⑧ "how much does recipient A's drain delay B" (per-recipient causality), which ADR-0125 asked, needs timestamps inside the drain loop and is not answered here. If this wording is removed, the next session misreads this number as that answer.
const FANOUT_CAVEAT: &str =
    "aggregate N-scaling only · per-recipient causality NOT measured (needs production hooks — user decision)";

/// ★Measured (4 independent runs)★: within one run the worst point is stable within ±0.2%, but between runs the fixed cost moves ~2x and the slope ~30%. Quoted coefficients will not reproduce; only the shape does.
const VARIANCE_NOTE: &str =
    "debug build · magnitudes vary ~2× across sessions; what reproduces is the shape (linear in batch size, worst ≈ cap−1, cap-full cheapest), not the coefficients";

/// ★Do not try to remove the hook★: this bin exists only with the `test-harness` feature on (seam session injection is behind the same feature). Keep the notice, not removal.
const INSTRUMENT_NOTE: &str =
    "test-harness feature is ON (this bin requires it) — the timed path carries hook lookups production never pays (mid-send RwLock read + accept-hook OnceLock lookup); lock-read scale, buried under the cross-session variance above";

// ── deterministic recipient seam ──

struct NoopStatus;
impl StatusSink for NoopStatus {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// declares `structured: true` to pass the reachability gate, and that value becomes the kernel's `LiveAgent::turn_signal` — the idle gate must hold for this recipient for the prefill to be parked.
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

/// A core without observation wiring — turn facts are planted directly with `ScriptedTurnFacts`, so no core table is needed.
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
    // The canonical name of an injected session without a profile = basename(cwd) (ADR-0101), so match the cwd tail to the name to make "visible name = address" hold.
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

// ── wiring ──

/// Production hands off to the flush lane here and another thread drains the queue; this harness does not start that lane, so prefilled parkings stay queued until just before measurement. The cost structure matches production (non-blocking enqueue), so timing is not distorted.
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

/// ★Made anew per iteration★ — reusing one grows the roster and ledger by the iteration count, so name resolution · `@all` sorting cost rises only in later iterations and mixes into the measurements.
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

/// The identity must be in the registry so the send path does not print the "discarded sender" warning (not a gate, but noise during measurement).
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

// ── statistics ────────────────────────────────────────────────────────────────────────────────────────

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

    /// ★0.1µs is noise theater (basis = `VARIANCE_NOTE`)★ — decimals would claim a reproducibility that does not exist.
    fn us_columns(&self) -> String {
        format!(
            "{:>MEDIAN_COL$.0} {:>10.0} {:>10.0}",
            self.median_ns / 1000.0,
            self.min_ns as f64 / 1000.0,
            self.max_ns as f64 / 1000.0
        )
    }
}

// ── ⑤ burst response latency ──────────────────────────────────────────────────────────────────────────

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
        let actually_parked = st.messaging.parked_len(recipient.id);
        if actually_parked != parked {
            return Err(format!(
                "prefill mismatch (rep {rep}): requested {parked} items but the queue has {actually_parked} — measurement premise collapsed"
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

/// If the copied constant silently diverges from the kernel, ⑤'s "worst case" label becomes false and the measurement is invalid.
fn verify_cap_boundary(points: &[BurstPoint]) -> Result<(), String> {
    let at = |n: usize| points.iter().find(|p| p.parked == n);
    if let Some(full) = at(MAILBOX_CAP) {
        if !full.row.contains("MAILBOX_FULL") {
            return Err(format!(
                "at {MAILBOX_CAP} parked items it is `{}`, not a bounce — the kernel MAILBOX_CAP is not {MAILBOX_CAP}",
                full.row
            ));
        }
    }
    if let Some(edge) = at(MAILBOX_CAP - 1) {
        if edge.row.contains("MAILBOX_FULL") {
            return Err(format!(
                "already bounced at {} parked items — the kernel MAILBOX_CAP is smaller than {MAILBOX_CAP}",
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

/// No turn facts are planted, so everyone is idle (positive-knowledge-only) — the drain for N recipients all happens inside this one call.
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
                "@all(N={n}, rep {rep}) delivery rows are {delivered_rows} — the premise is delivery to all N recipients after excluding the sender"
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

// ── report ────────────────────────────────────────────────────────────────────────────────────────

fn profile_label() -> &'static str {
    if cfg!(debug_assertions) {
        "debug (unoptimized + debug-assertions)"
    } else {
        "release (optimized)"
    }
}

fn main() {
    println!("=== drain-latency — ADR-0125 synchronous drain measurement (spec §7 ⑤·⑧) ===");
    println!("measurement only — no threshold assertions (spec §7 forbids them for both ⑤ and ⑧)");
    println!();
    println!("  stage        : deterministic seam recipient — no real claude · no real PTY");
    println!("  timed region : one engram_dashboard_daemon::control::ingress::handle_send call");
    println!("  excluded     : stage assembly · session insertion · mailbox prefill · reply reading · statistics");
    println!(
        "               : ★real stdin writes are excluded too★ — the seam stands in with an in-memory capture (production does a child"
    );
    println!("                 pipe/ConPTY blocking write_all+flush). The ⑤ notice below is the source of truth.");
    println!("  mailbox cap  : {MAILBOX_CAP} (copy of messaging mailbox.rs `MAILBOX_CAP` — the boundary is re-measured every run)");
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

    println!("⑤ burst response latency — {BURST_CAVEAT}");
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
                println!("  {parked:>6} ★measurement invalid★ {e}");
                failed = true;
            }
        }
    }
    if let Err(e) = verify_cap_boundary(&burst_points) {
        println!("  ★CAP DRIFT★ {e}");
        failed = true;
    }
    println!();
    println!("⑤ scope notice (quote as is): {BURST_CAVEAT}");
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
                println!("  {n:>6} ★measurement invalid★ {e}");
                failed = true;
            }
        }
    }
    println!();
    println!("⑧ scope notice (quote as is): {FANOUT_CAVEAT}");

    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── statistics ────────────────────────────────────────────────────────────────

    #[test]
    fn median_of_odd_sample_is_the_middle_value() {
        let s = Stats::of(&[30, 10, 20, 50, 40]).expect("has samples");
        assert_eq!(s.median_ns, 30.0);
        assert_eq!(s.min_ns, 10);
        assert_eq!(s.max_ns, 50);
    }

    #[test]
    fn median_of_even_sample_averages_the_middle_pair() {
        let s = Stats::of(&[40, 10, 30, 20]).expect("has samples");
        assert_eq!(s.median_ns, 25.0);
        assert_eq!(s.min_ns, 10);
        assert_eq!(s.max_ns, 40);
    }

    #[test]
    fn single_sample_collapses_to_one_value() {
        let s = Stats::of(&[7]).expect("has samples");
        assert_eq!((s.median_ns, s.min_ns, s.max_ns), (7.0, 7, 7));
    }

    #[test]
    fn empty_sample_has_no_stats() {
        assert!(Stats::of(&[]).is_none());
    }

    // ── prefill ──────────────────────────────────────────────────────────────

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

        assert_eq!(st.messaging.parked_len(recipient.id), 7);
        assert_eq!(
            written_len(&recipient),
            0,
            "while in a turn it is not injected (only parked)"
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
        assert_eq!(written_len(&recipient), 5, "4 stale items + 1 this time");
        assert_eq!(st.messaging.parked_len(recipient.id), 0);
    }
}
