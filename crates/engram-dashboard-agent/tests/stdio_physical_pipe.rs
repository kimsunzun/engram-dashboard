//! ③ 물리 OS-pipe 계층 통합테스트 — 실 StdioTransport + 실 OS 파이프로 배달 정확성을 검증한다.
//!
//! ADR-0088 Stage 1 의 데몬 seam 테스트(control_send.rs)는 `SeamTransport` 가 이미 완결된 Vec 을
//!   원자 `push` 로 캡처하므로 **물리 파이프 계층을 우회**한다 — 이 파일이 그 "반환 follow-up",
//!   즉 운영 StdioTransport 의 물리 stdin write 경로(`stdin.lock()` + `write_all` + `flush`)를 덮는다.
//!
//! ★그 경로를 도는 주체가 바뀌었다 — 이 파일의 항목들이 그 위에 선다★: `send_input` 은 이제 유계 큐에
//!   담기만 하고, 물리 쓰기는 **전담 라이터 스레드**가 한다(`transport::input_queue`). 그래서 이 파일은
//!   `transport.start()` 를 반드시 부르고(라이터가 거기서 뜬다), `send_input` 의 `Ok` 를 "배달됐다" 가
//!   아니라 **"받았다"** 로 읽는다. 배달 여부는 언제나 자식의 echo 로 잰다.
//!
//! ★Windows 전용★: 자식이 powershell 이라 Windows 에서만 컴파일·실행한다(프로젝트 전제).
// ADR-0088
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::transport::input_queue::INPUT_QUEUE_MAX_BYTES;
use engram_dashboard_agent::transport::stdio::StdioTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::OutputSink;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload, PtyError,
    SinkError, SinkId, StatusSink,
};

// ── 수집 sink ───────────────────────────────────────────────────────────────────────────
#[derive(Clone)]
struct CollectingSink {
    id: SinkId,
    chunks: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl CollectingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }
    /// 스트림 순서 = 자식 stdout 순서.
    fn concat_ordered(&self) -> Vec<u8> {
        let mut v = self.chunks.lock().unwrap().clone();
        v.sort_by_key(|(seq, _)| *seq);
        let mut out = Vec::new();
        for (_, b) in v {
            out.extend_from_slice(&b);
        }
        out
    }
    fn total_len(&self) -> usize {
        self.chunks
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| b.len())
            .sum()
    }
}
impl OutputSink for CollectingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // decoder=None 경로라 payload 는 항상 Bytes(TerminalBytes 직통). Event 는 안 온다.
        if let OutputPayload::Bytes(b) = frame.payload {
            self.chunks.lock().unwrap().push((frame.seq, b.to_vec()));
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

// ── status sink: 이 파일은 상태 전이를 단언하지 않으므로 no-op ──────────────────────────
struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        cwd: PathBuf::from("."),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 1 — 물리 OS-pipe 동시 write 무인터리브 (stdin.lock() 회귀 그물)
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★증명한다★: 물리 파이프 계층의 **응용계층(application-layer) 직렬화** — 한 논리 메시지가 여러 OS
///   write 로 갈려도 다른 writer 의 write 가 그 사이에 끼어들지 못한다. ★그 직렬화를 지는 주체가
///   바뀌었다★: 예전엔 `send_input` 이 `stdin.lock()` 을 write_all+flush 내내 쥐는 것이었고, 지금은
///   **한 덩이가 큐의 한 칸이고 그것을 빼서 쓰는 스레드가 하나뿐**인 것이다(`transport::input_queue`).
///   재는 성질과 이 항목의 오라클은 그대로다. 이어붙인 스트림이 **정확히 N 개의
///   연속 런**(fill 바이트당 1개, 각 길이 정확히 L)이면: 인터리브 없음(있으면 런이 N 개 초과) +
///   유실 없음(총량·각 런 길이 정확) + 중복/치환 없음(각 바이트값이 정확히 1런).
///   ▷ 검증된 회귀 형태 = **"한 논리 메시지를 배타 락 없이 여러 OS write 로 쓰는 것"**. 경험적 확인:
///     락을 드롭하고 청크 단위로 쓰도록 변이(chunked-writes-lock-dropped)하니 런이 6개로 늘어(N=4
///     초과) 테스트가 실패했다 → 이 형태의 회귀는 잡는다.
/// ★증명하지 않는다★:
///   (1) 락이 **없을 때 인터리브가 실제로 발생함**을 강제하진 못한다 — 현 구현(락 존재) 하에서
///       **인터리브 부재**를 증명할 뿐. 느린 reader·backpressure·L≥파이프버퍼 설계로 "락 제거 시"
///       인터리브 창을 최대화하도록 짰다(회귀 그물로서 최선).
///   (2) **락 없는 단일-WriteFile 구현**(각 256KiB 메시지를 1회의 blocking WriteFile 로 발화)은 이
///       테스트가 잡는다고 **주장하지 않는다** — 그 경우 NPFS(anonymous byte-mode pipe)가 write 요청
///       전체를 커널에서 직렬화해 런이 여전히 N 개일 수 있다. 바이트모드 파이프의 단일 write 요청
///       원자성은 **문서화되지 않은 커널 동작**이라 어느 쪽도 단언하지 않는다. 프로덕션을 못 건드려
///       외부에서 다중-syscall write 를 강제할 수 없으므로 주장 범위를 응용계층 직렬화로 한정한다.
#[test]
fn physical_pipe_concurrent_sends_no_interleave() {
    // 느린 reader echo 자식(8KiB 청크마다 1ms) → 파이프가 backpressure 로 찬다. backpressure 는 write 가
    //   여러 OS write 로 갈리는 구현(검증된 회귀 형태 = 배타 락 없는 chunked write)에서 **인터리브 창을
    //   넓힌다**. Console 표준스트림 바이트 루프라 cmd.exe 로는 불가(byte-level 제어 없음) — powershell 사용.
    let child_script = "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$stdout.Write($buf,0,$n);$stdout.Flush();Start-Sleep -Milliseconds 1}";

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", child_script]),
        false, // 평문 stdio — TerminalBytes 직통(decoder=None)
        None,
    )
    .expect("open");
    let transport = Arc::new(transport);

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // L 은 OS 파이프 버퍼를 넉넉히 초과 — 락 제거 시 인터리브 창을 최대화한다.
    //   256KiB × 4 = 1MiB 총량, 느린 reader(8KiB/1ms ≈ 8MiB/s)로도 벽시계 ~수초 안.
    const N: usize = 4;
    const L: usize = 256 * 1024;

    // ★FIX 3★: 모든 실패/타임아웃/panic-join/spawn-실패 경로는 panic 전 transport.shutdown() 으로 blocked
    //   writer·자식·pump 를 정리한다(다른 테스트 동시 실행 중 누수 방지).
    macro_rules! fail {
        ($($arg:tt)*) => {{
            transport.shutdown();
            panic!($($arg)*);
        }};
    }

    let barrier = Arc::new(Barrier::new(N));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let t = transport.clone();
        let b = barrier.clone();
        // ★thread-creation 실패 대응(round-3 MEDIUM)★: raw spawn 은 OS 스레드 생성 실패 시 즉시 panic 해
        //   shutdown 없이 all-panic-paths-cleanup 계약을 깬다 → Builder::spawn 으로 Err 를 받아 fail!(shutdown
        //   후 panic) 로 라우팅. 단, 이미 spawn 된 writer 들은 send_input 이전 단계인 Barrier::wait 에 park 돼
        //   있어(N-party barrier 가 N 명 도달 전 열리지 않음) shutdown 의 파이프-kill 로도 깨어나지 못한다 —
        //   detach 된 채 남아 프로세스 종료 때 회수된다(spawn 실패는 이미 큰 실패라 여기서 loud panic 으로 종결).
        let h = match std::thread::Builder::new().spawn(move || {
            let fill = b'A' + i as u8;
            let payload = vec![fill; L];
            b.wait(); // 진입 정렬 — 동시 진입으로 경합 창 최대화.
            t.send_input(InputEvent::Raw(payload))
        }) {
            Ok(h) => h,
            Err(e) => fail!("writer {i} OS 스레드 생성 실패: {e} — 앞서 spawn 된 writer 는 Barrier 에 park(detach, 프로세스 종료 시 회수)"),
        };
        handles.push(h);
    }
    // ★watchdog(FIX 2)★: 자식이 stdin drain 을 멈추면 write_all 이 파이프 backpressure 로 락 쥔 채
    //   블록 → 순진한 `join()` 은 영영 안 돌아온다(뒤의 출력 데드라인엔 도달조차 못 함).
    //   `is_finished()` 를 데드라인까지 폴링해 hang 을 큰 실패로 전환한다.

    let join_deadline = Instant::now() + Duration::from_secs(120);
    for (i, h) in handles.into_iter().enumerate() {
        while !h.is_finished() {
            if Instant::now() >= join_deadline {
                fail!("writer {i} 가 120s 안에 반환 못함 — write_all 이 파이프 backpressure 로 hang(자식 echo drain 정지 의심)");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let r = match h.join() {
            Ok(r) => r,
            Err(_) => fail!("writer {i} 스레드가 panic — send_input 내부 실패"),
        };
        if !r.is_ok() {
            fail!("writer {i} send_input 실패: {r:?}");
        }
    }

    let expected_total = N * L;
    let deadline = Instant::now() + Duration::from_secs(120);
    while sink.total_len() < expected_total {
        if Instant::now() >= deadline {
            let got = sink.total_len();
            transport.shutdown();
            panic!("120s 안에 echo 총량이 기대치({expected_total})에 도달 못함(현재 {got}). 자식 echo hang 의심.");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let stream = sink.concat_ordered();
    if stream.len() != expected_total {
        fail!(
            "echo 총 바이트가 N*L 과 불일치(유실/초과): got {}, want {expected_total}",
            stream.len()
        );
    }
    let runs = run_length_encode(&stream);
    if runs.len() != N {
        fail!(
            "런 개수가 N({N})이 아님 — 인터리브 발생(stdin.lock() 회귀). 런 요약: {:?}",
            summarize_runs(&runs)
        );
    }
    // 런의 순서는 무관(경합).
    let mut seen_bytes = std::collections::HashSet::new();
    for (byte, len) in &runs {
        if *len != L {
            fail!("바이트 {byte:#x} 의 런 길이가 L({L})이 아님({len}) — 유실 또는 분절");
        }
        if !seen_bytes.insert(*byte) {
            fail!("바이트 {byte:#x} 가 두 번 이상 런으로 나타남 — 중복/분절(인터리브)");
        }
    }
    for i in 0..N {
        let fill = b'A' + i as u8;
        if !seen_bytes.contains(&fill) {
            fail!("fill 바이트 {fill:#x} 가 스트림에 없음 — writer {i} 유실");
        }
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}

fn run_length_encode(bytes: &[u8]) -> Vec<(u8, usize)> {
    let mut runs: Vec<(u8, usize)> = Vec::new();
    for &b in bytes {
        match runs.last_mut() {
            Some((val, len)) if *val == b => *len += 1,
            _ => runs.push((b, 1)),
        }
    }
    runs
}

/// 인터리브 시 런이 폭증하므로 앞 20개만.
fn summarize_runs(runs: &[(u8, usize)]) -> Vec<(char, usize)> {
    runs.iter()
        .take(20)
        .map(|(b, l)| (*b as char, *l))
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 2 — 상한 초과는 **거절**이다(조용히 버리지 않는다)
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★한때 이 자리에 있던 항목(`physical_pipe_partial_write_then_err_surfaces_as_err`)은 **더 이상
///   성립하지 않는다 — 계약이 바뀌었기 때문이지 약해졌기 때문이 아니다**★.
///   옛 항목은 「`send_input` 이 호출 스레드에서 `write_all` 을 돌린다」에 기대어 「prefix 가 물리적으로
///   나간 **뒤에도** 그 호출이 `Err` 로 돌아온다」를 쟀다. 지금 `send_input` 은 OS 를 건드리지 않고 유계
///   큐에 담기만 하므로(모듈 = `transport::input_queue`) **그 호출이 볼 수 있는 실패는 「받지 않았다」뿐**
///   이다. 옛 항목이 쟀던 나머지 절반(「받아 둔 뒤의 실패를 어떻게 신고하나」)은 Test 3 으로 갔다.
///
/// ★이 항목이 증명한다★: 큐 상한을 넘는 페이로드는 **통째로 거절**되고(`Err`), ★그 거절이 조용한
///   유실이 아니라는 직접 증거로 **자식이 한 바이트도 못 받는다**★. 상한 초과를 부분 수용하거나
///   앞부분만 쓰고 `Ok` 로 돌려주는 회귀는 아래 두 단언 중 하나에 반드시 걸린다.
/// (ADR-0190 의 처분 — "넘으면 그 호출이 `Err` 로 돌아간다 — 버리지 않는다" — 의 물리 계층 판.)
#[test]
fn physical_pipe_oversized_payload_is_refused_whole() {
    // 자식은 stdin 을 계속 읽어 echo 한다 — 즉 **배달될 수 있었는데 안 됐다**를 재는 자리다
    // (자식이 안 읽어서 못 간 것이 아니다).
    let child_script = "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$stdout.Write($buf,0,$n);$stdout.Flush()}";

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", child_script]),
        false,
        None,
    )
    .expect("open");

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    let over = vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1];
    let result = transport.send_input(InputEvent::Raw(over));
    if !matches!(result, Err(PtyError::WriteFailed(_))) {
        transport.shutdown();
        panic!("상한 초과는 WriteFailed 로 거절돼야 한다(Ok 위장 금지): {result:?}");
    }

    // ★조용한 유실이 아님의 직접 증거★: 거절된 뒤 1s 동안 자식이 아무것도 되울리지 않는다.
    //   부분 수용 회귀라면 그 앞부분이 echo 로 돌아와 여기서 잡힌다.
    std::thread::sleep(Duration::from_secs(1));
    let leaked = sink.total_len();

    // 대조군 — 상한 아래는 정상 배달된다(위 0 이 "통로가 애초에 죽었다"가 아님을 배제).
    const PROBE: &[u8] = b"under-the-bound\n";
    transport
        .send_input(InputEvent::Raw(PROBE.to_vec()))
        .expect("상한 아래 페이로드는 받아야 한다");
    let deadline = Instant::now() + Duration::from_secs(30);
    while sink.total_len() < PROBE.len() {
        if Instant::now() >= deadline {
            let got = sink.total_len();
            transport.shutdown();
            panic!(
                "대조군이 30s 안에 echo 되지 않음({got}) — 통로가 애초에 죽어 있어 위 0 이 공허함"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    if leaked != 0 {
        transport.shutdown();
        panic!("거절된 페이로드의 {leaked} 바이트가 자식에게 갔다 — 부분 수용(조용한 유실) 회귀");
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 3 — 받아 둔 뒤 실패한 쓰기는 **다음 호출**이 사유를 들고 거절한다
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★증명한다★:
///   (a) 받아 둔 바이트가 **자식이 죽어 못 나가면** 통로가 닫히고, ★**다음** `send_input` 이 사유를
///       들고 `Err` 로 돌아온다★ — 실패가 조용히 삼켜지지 않는다. (받아 둔 그 호출에게 돌려줄 길이
///       없다는 것이 이 설계의 알려진 대가이고, 이 항목이 그 대가의 **경계**를 못 박는다: 다음
///       호출부터는 반드시 정직하다.) ★어느 스레드가 먼저 알아채느냐는 경합이라 안 잰다★ — 사유
///       문자열에 기대지 않는 근거는 아래 (a) 단언 자리의 주석.
///   (b) 실패 전에 나간 prefix 가 **정확히 payload 의 앞 K 바이트**다 — 자식이 첫 read 전에 죽었거나
///       라이터가 아무것도 쓰기 전에 실패했다면 이 등식이 깨진다(vacuous pass 차단). 옛 Test 2 의
///       비반복 페이로드 오라클을 그대로 물려받는다.
/// ★옛 Test 2 와 갈리는 곳은 (a) 하나뿐이다★ — 거기선 **그 호출**이 Err 였고 여기선 **다음 호출**이다.
#[test]
fn physical_pipe_write_failure_after_acceptance_surfaces_on_the_next_call() {
    // K = 64KiB 를 읽고 각 청크를 echo 한 뒤 종료. 종료 후 파이프가 끊겨 남은 write_all 이 실패한다.
    //   마지막 read 를 남은 (K - total) 로 cap 해 총 소비·echo 를 정확히 K 로 맞춘다.
    const K: usize = 64 * 1024;
    let child_script = format!(
        "\
$stdin=[Console]::OpenStandardInput();\
$stdout=[Console]::OpenStandardOutput();\
$buf=New-Object byte[] 4096;\
$total=0;\
while($total -lt {K}){{\
$cap=[Math]::Min($buf.Length,{K}-$total);\
$n=$stdin.Read($buf,0,$cap);\
if($n -le 0){{break}};\
$stdout.Write($buf,0,$n);$stdout.Flush();\
$total+=$n}};\
exit 0"
    );

    let (transport, _pid) = StdioTransport::open(
        &spec("powershell.exe", &["-NoProfile", "-Command", &child_script]),
        false, // 평문 stdio — TerminalBytes 직통(decoder=None)
        None,
    )
    .expect("open");

    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink),
        TurnWiring::detached(),
    ));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // 1MiB — K + 파이프 버퍼를 압도하되 큐 상한(2MiB) 아래. ★옛 8MiB 에서 내린 것은 상한 때문이고,
    //   이 항목이 재는 성질(자식이 K 만 먹고 죽어 남은 write 가 실패한다)은 그대로다★ — 1MiB 는 기본
    //   파이프 버퍼의 10배 이상이라 자식 종료 시 반드시 미전송 잔여가 남는다.
    //   ★비반복 스트림★: 고정 시드 xorshift64(seed=0x9E37_79B9_7F4A_7C15). 결정적이고 어떤 오프셋의
    //   K-창도 payload[..K] 와 겹치지 않아, prefix 가 잘못된 오프셋에서 쓰이는 회귀를 (b) 가 잡는다.
    const PAYLOAD_LEN: usize = 1024 * 1024;
    let payload: Vec<u8> = {
        let mut v = Vec::with_capacity(PAYLOAD_LEN);
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        while v.len() < PAYLOAD_LEN {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            v.extend_from_slice(&state.to_le_bytes());
        }
        v.truncate(PAYLOAD_LEN);
        v
    };
    let expected_prefix = payload[..K].to_vec();

    // 모든 실패 경로는 panic 전 transport.shutdown() 으로 자식·pump·라이터를 정리한다.
    macro_rules! fail {
        ($($arg:tt)*) => {{
            transport.shutdown();
            panic!($($arg)*);
        }};
    }

    // ★self-check(기계 검증) — 무-앨리어싱★: 어떤 시프트 s 에서도 payload[s..s+K] != payload[..K].
    //   같은 창이 존재하면 (b) 오라클이 그 오프셋에서 눈멀어 회귀를 못 잡는다. early-exit 덕에 빠르다.
    let p0 = payload[0];
    for s in 1..=(payload.len() - K) {
        if payload[s] == p0 && payload[s..s + K] == expected_prefix[..] {
            fail!("무-앨리어싱 self-check 실패: shift s={s} — 시드/생성기를 바꿔 재생성 필요");
        }
    }

    // ★이 호출은 즉시 `Ok` 다 — 그것이 이 라운드의 요점이다★(부른 쪽이 OS 쓰기를 기다리지 않는다).
    let accept_start = Instant::now();
    if let Err(e) = transport.send_input(InputEvent::Raw(payload)) {
        fail!("상한 아래 페이로드는 받아야 한다: {e:?}");
    }
    let accept_elapsed = accept_start.elapsed();
    if accept_elapsed >= Duration::from_secs(2) {
        fail!("send_input 이 {accept_elapsed:?} 걸렸다 — 호출자가 OS 쓰기에 매달린 회귀");
    }

    // (b) prefix 가 실제로 물리 배달됐다 — 공허한 통과 차단.
    let prefix_deadline = Instant::now() + Duration::from_secs(30);
    while sink.total_len() < K {
        if Instant::now() >= prefix_deadline {
            fail!(
                "30s 안에 echo prefix 가 K({K})에 도달 못함(현재 {}) — prefix 미배달(vacuous pass 의심)",
                sink.total_len()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let echoed = sink.concat_ordered();
    if echoed.len() != K {
        fail!(
            "echo 총량이 K({K})가 아님({}) — 자식이 정확히 K 를 소비/echo 하지 않음",
            echoed.len()
        );
    }
    if echoed != expected_prefix {
        fail!("echo prefix 가 payload[..K] 와 불일치 — 물리 배달된 게 앞쪽 prefix 가 아님(무결성 위반)");
    }

    // (a) 자식이 죽어 통로가 닫힌다 → **다음** 호출이 사유를 들고 거절.
    //     ★폴링인 이유★: 닫는 시점은 다른 스레드의 일이라 이 스레드와 동기가 아니다. 닫힘이 관측될
    //     때까지 기다리되, 안 닫히면(=실패를 삼키는 회귀) 데드라인이 큰 실패로 바꾼다.
    // ★★어느 사유가 이길지는 **경합이고, 둘 다 참이다** — 그래서 사유 문자열로 단언하지 않는다★★:
    //   자식이 죽으면 ⓐ 라이터의 블록된 write 가 에러로 풀리는 것과 ⓑ pump 가 stdout EOF 를 보고
    //   `WriterStop` 으로 닫는 것이 **동시에** 시작되고, 실측에서는 ⓑ 가 먼저 이겼다(2026-09-17).
    //   `InputQueue::close` 는 첫 사유를 지키므로 그때 문구는 "스트림이 끝났다" 다. 이 항목이 재려는
    //   성질은 **「받아 둔 뒤의 실패가 조용히 삼켜지지 않는다」**이지 어느 스레드가 먼저 알아채느냐가
    //   아니므로, 단언은 「거절된다」 + 「그 사유가 상한이 아니다」로 잡는다(상한이면 이 항목이 재려던
    //   것과 다른 이유로 통과하는 셈이라 그것만 배제한다).
    let close_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match transport.send_input(InputEvent::Raw(b"after\n".to_vec())) {
            Err(PtyError::WriteFailed(msg)) => {
                if msg.contains("상한") {
                    fail!("거절 사유가 상한이다 — 통로가 닫혀서 거절된 것이 아니다: {msg}");
                }
                break;
            }
            Ok(()) => {
                if Instant::now() >= close_deadline {
                    fail!("30s 안에 통로가 닫히지 않음 — 자식의 죽음이 조용히 삼켜졌다");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(other) => fail!("WriteFailed 여야: {other:?}"),
        }
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}
