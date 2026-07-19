//! ③ 물리 OS-pipe 계층 통합테스트 — 실 StdioTransport + 실 OS 파이프로 배달 정확성을 검증한다.
//!
//! ADR-0088 Stage 1 의 데몬 seam 테스트(control_send.rs)는 `SeamTransport` 가 이미 완결된 Vec 을
//!   원자 `push` 로 캡처하므로 **물리 파이프 계층을 우회**한다. 그래서 두 축이 seam 에선 검증
//!   불가능해 "반환 follow-up" 으로 남았다 — 이 파일이 그 follow-up 이다:
//!   - Test 1: `send_input` 이 `stdin.lock()` 을 write_all+flush 내내 쥐어 **동시 write 의 OS-pipe
//!     바이트 인터리브를 막는다**(stdio.rs ~319). seam 은 완결 Vec 을 받으므로 이 락을 지워도 안
//!     걸린다 → 실 파이프·느린 reader·backpressure 로 인터리브 창을 최대화한 하네스가 필요.
//!   - Test 2: 실제 OS write 가 **prefix 를 물리적으로 쓴 뒤 Err** 를 내도 `write_all` 계약상
//!     `Ok` 로 위장되지 않고 `Err` 로 표면화됨(부분 배달이 성공으로 보고되지 않음). seam 은 push
//!     **전에** 통째로 Err 를 내는 all-or-nothing 모사라 "prefix 만 쓰이고 실패" 자체가 없다.
//!
//! 이 파일이 **커버하는 계층** = 운영 StdioTransport 의 물리 stdin write 경로(`stdin.lock()` +
//!   `write_all` + `flush`). 데몬 seam 테스트가 커버하는 계층(ingress→registry→session 조립·
//!   exact-once entry)과 **상보적**이다 — 둘을 합쳐야 배달 경로 전체가 덮인다.
//!
//! ★Windows 전용★: 자식이 powershell/cmd 라 Windows 에서만 컴파일·실행한다(프로젝트 전제).
//!   stdio_smoke.rs 와 동형(cmd.exe 픽스처). 비Windows 는 빈 파일.
// ADR-0088
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::output_core::OutputCore;
use engram_dashboard_core::agent::transport::stdio::StdioTransport;
use engram_dashboard_core::agent::transport::AgentTransport;
use engram_dashboard_core::agent::types::OutputSink;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload, PtyError,
    SinkError, SinkId, StatusSink,
};

// ── 수집 sink: pump 가 core 로 흘린 바이트를 seq 순서대로 이어붙여 보관 ────────────────
//   stdio_smoke.rs 의 RecordingSink 와 동형이되, 이 파일은 바이트 총량/런렝스만 보므로 최소화.
#[derive(Clone)]
struct CollectingSink {
    id: SinkId,
    /// (seq, bytes). pump 가 emit 한 순서(seq 단조 증가)대로 쌓인다.
    chunks: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl CollectingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }
    /// seq 오름차순으로 정렬해 이어붙인 전체 바이트(스트림 순서 = 자식 stdout 순서).
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

/// ── ADR-0088 Stage 1 반환 follow-up 1: 물리 OS-pipe 바이트 무인터리브 ──
///
/// N 개 OS 스레드가 각기 **고유 fill 바이트**로 채운 큰 페이로드를 동시에 `send_input` 한다. 자식은
///   stdin 을 작은 청크로 **느리게** 읽어 되받아치는 echo 프로세스라 파이프가 backpressure 로 찬다.
///   backpressure 는 write 를 여러 OS write 로 갈리게 하는 구현(배타 락 없는 chunked write)에서 두
///   writer 의 바이트가 한 write 창 안에서 섞일 인터리브 창을 넓힌다(현 구현엔 강제 아님 — 아래
///   ★증명하지 않는다★). echo 로 돌아온 stdout 을 seq 순서로 이어붙여 런렝스 분석한다.
///
/// ★증명한다★: 물리 파이프 계층의 **응용계층(application-layer) 직렬화** — `send_input` 이
///   `stdin.lock()` 을 write_all+flush 내내 쥐므로(stdio.rs ~319) 한 논리 메시지가 여러 OS write 로
///   갈려도 다른 writer 의 write 가 그 사이에 끼어들지 못한다. 이어붙인 스트림이 **정확히 N 개의
///   연속 런**(fill 바이트당 1개, 각 길이 정확히 L)이면: 인터리브 없음(있으면 런이 N 개 초과) +
///   유실 없음(총량·각 런 길이 정확) + 중복/치환 없음(각 바이트값이 정확히 1런). 이 계층은 데몬
///   seam 테스트가 우회하는 물리 계층이다.
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
    // 느린 reader echo 자식: stdin 을 8KiB 청크로 읽어 stdout 으로 그대로 되돌리고 청크마다 1ms
    //   잠깐 쉰다 → reader 가 느려 파이프가 backpressure 로 찬다. backpressure 는 write 가 여러 OS
    //   write 로 갈리는 구현(검증된 회귀 형태 = 배타 락 없는 chunked write)에서 **인터리브 창을 넓힌다**.
    //   현 구현은 write_all 을 락 쥔 채 하므로 이걸로 다중 OS write 분할이 강제되진 않는다(Windows 의
    //   blocking WriteFile 은 backpressure 와 무관하게 단일 요청 — round-1 FIX-4 근거). Console 표준스트림
    //   바이트 루프라 cmd.exe 로는 불가(byte-level 제어 없음) — powershell 사용.
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
    let core = Arc::new(OutputCore::new(Uuid::new_v4(), 0, Arc::new(NoopStatusSink)));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // N 개 writer + 각 L 바이트. 큰 L(OS 파이프 버퍼를 넉넉히 초과) + 느린 reader 는 **여러 write 를
    //   내는 구현**(검증된 회귀 형태 = 배타 락 없는 chunked write)에 대해 인터리브 창/노출을 최대화한다.
    //   현 단일 write_all 구현의 강제 분할을 뜻하진 않는다(blocking WriteFile 은 단일 요청 — :135-136 참조).
    //   256KiB × 4 = 1MiB 총량, 느린 reader(8KiB/1ms ≈ 8MiB/s)로도 벽시계 ~수초 안.
    const N: usize = 4;
    const L: usize = 256 * 1024;

    // ★FIX 3★: 모든 실패/타임아웃/panic-join/spawn-실패 경로는 panic 전 transport.shutdown() 으로 blocked
    //   writer·자식·pump 를 정리한다(다른 테스트 동시 실행 중 누수 방지). `fail!` = shutdown 후 panic 하는
    //   로컬 매크로. spawn 루프·join 루프보다 앞에 정의해 OS 스레드 생성 실패(Builder::spawn Err)와
    //   writer-thread panic(join Err) 둘 다 이 경로를 타게 한다.
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
            // 스레드 i = 고유 fill 바이트(b'A'+i)를 L 개. 값이 겹치지 않아 런렝스에서 소유 writer 식별 가능.
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
    // 모든 send 는 Ok(무유실·무에러의 1차 신호). ★watchdog(FIX 2, stdio.rs FIX-1 테스트와 동형)★:
    //   자식이 stdin drain 을 멈추면 write_all 이 파이프 backpressure 로 락 쥔 채 블록 → 순진한
    //   `join()` 은 영영 안 돌아온다(뒤의 출력 데드라인엔 도달조차 못 함). `is_finished()` 를 데드라인까지
    //   폴링해 hang 을 큰 실패로 전환하고, 만료 시 shutdown(파이프 끊어 blocked write 해제 + 자식 kill)
    //   → 리소스 누수 없이 panic(FIX 3).

    let join_deadline = Instant::now() + Duration::from_secs(120);
    for (i, h) in handles.into_iter().enumerate() {
        while !h.is_finished() {
            if Instant::now() >= join_deadline {
                fail!("writer {i} 가 120s 안에 반환 못함 — write_all 이 파이프 backpressure 로 hang(자식 echo drain 정지 의심)");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // writer 스레드가 panic 했으면 join 이 Err — 순진한 expect() 는 shutdown 없이 panic 해
        //   all-panic-paths-cleanup 계약을 깬다. fail! 로 라우팅해 자식/pump 를 정리한다.
        let r = match h.join() {
            Ok(r) => r,
            Err(_) => fail!("writer {i} 스레드가 panic — send_input 내부 실패"),
        };
        if !r.is_ok() {
            fail!("writer {i} send_input 실패: {r:?}");
        }
    }

    // 오라클: echo 로 돌아온 총 바이트가 N*L 이 될 때까지 폴링(넉넉한 데드라인 — hang 은 크게 실패).
    let expected_total = N * L;
    let deadline = Instant::now() + Duration::from_secs(120);
    while sink.total_len() < expected_total {
        if Instant::now() >= deadline {
            let got = sink.total_len();
            transport.shutdown(); // 자식/pump 정리(FIX 3 — 누수 방지)
            panic!("120s 안에 echo 총량이 기대치({expected_total})에 도달 못함(현재 {got}). 자식 echo hang 의심.");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // 런렝스 분석: 이어붙인 스트림은 정확히 N 개의 연속 런이어야 한다(각 fill 바이트, 각 길이 L).
    //   실패 경로는 위에서 정의한 `fail!`(shutdown 후 panic)로 자식/pump 를 정리한다.
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
    // 각 런: 길이 정확히 L, 바이트값 전부 distinct(치환/중복 없음). 순서는 무관(경합).
    let mut seen_bytes = std::collections::HashSet::new();
    for (byte, len) in &runs {
        if *len != L {
            fail!("바이트 {byte:#x} 의 런 길이가 L({L})이 아님({len}) — 유실 또는 분절");
        }
        if !seen_bytes.insert(*byte) {
            fail!("바이트 {byte:#x} 가 두 번 이상 런으로 나타남 — 중복/분절(인터리브)");
        }
    }
    // 발신한 fill 바이트 집합(A..A+N)과 정확히 일치.
    for i in 0..N {
        let fill = b'A' + i as u8;
        if !seen_bytes.contains(&fill) {
            fail!("fill 바이트 {fill:#x} 가 스트림에 없음 — writer {i} 유실");
        }
    }

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}

/// 연속 동일 바이트를 (바이트값, 길이) 런 목록으로 압축.
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

/// 실패 메시지용 런 요약(앞 20개만 — 인터리브 시 런이 폭증하므로 절단).
fn summarize_runs(runs: &[(u8, usize)]) -> Vec<(char, usize)> {
    runs.iter()
        .take(20)
        .map(|(b, l)| (*b as char, *l))
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// Test 2 — 실 부분 write 후 Err (부분 배달이 Ok 로 위장되지 않음)
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ── ADR-0088 Stage 1 반환 follow-up 2: 실 OS write prefix-후-Err 표면화 ──
///
/// 자식은 stdin 을 **정확히 K 바이트**만 읽되 읽은 청크를 **stdout 으로 그대로 echo** 한 뒤 종료한다
///   (마지막 read 는 K-소비량으로 cap 해 총 소비/echo 를 정확히 K 로 맞춤 — Polish). 그보다 훨씬 큰
///   페이로드(8MiB — 기존 FIX-1 테스트와 동일 규모)를 1회 `send_input`. OS 는 prefix(자식이 소비한 K
///   + 파이프 버퍼)를 물리적으로 받아들인 뒤, 자식 종료로 파이프가 끊겨 진행 중이던 `write_all` 이
///   실패한다. 페이로드는 **고정 시드 PRNG(xorshift64) 로 생성한 비반복 스트림**이라 echo 로 돌아온
///   prefix 의 바이트 **정체(identity)** 가 의미를 가진다(균일 fill 이면 "K 바이트 도착"만 알 뿐 그게
///   진짜 앞쪽 prefix 인지 구분 불가). 어떤 잘못된 오프셋에서 쓰였든 그 창의 바이트가 payload[..K] 와
///   달라 (b) 등식이 회귀를 잡는다 — 이 무-앨리어싱(non-aliasing) 성질은 send 전에 **런타임에서 기계로
///   검증**한다(아래 self-check).
///
/// ★증명한다★:
///   (a) `send_input` 이 `Err(PtyError::WriteFailed(_))` 를 반환한다. std `write_all` 계약상 prefix 가
///       **물리적으로 쓰였음에도** 호출이 Err 로 표면화된다.
///   (b) echo 로 수집된 바이트가 **정확히 payload 의 앞 K 바이트와 일치**한다. K 바이트가 실제로
///       파이프를 통과해 자식이 소비·되돌렸다는 = **prefix 가 실패 전에 물리적으로 배달됐다는** 직접
///       증거다(자식이 첫 read 전에 죽었거나 transport 가 아무것도 쓰기 전에 실패했다면 이 등식이
///       깨진다 → 아래 "vacuous pass" 를 차단).
///   종합: **prefix 가 물리적으로 배달됐음에도 호출은 여전히 Err 로 표면화 ⇒ 부분 배달이 절대 `Ok` 로
///   보고되지 않는다.** 이는 데몬 오라클 4(`stage1_lifecycle_write_error_single_failure_no_partial_dup`)
///   의 물리 계층 상보물이다: 그 seam 은 바이트가 **한 톨도 움직이기 전에** Err 를 내는 all-or-nothing
///   모사인 반면, 여기선 실제 prefix 바이트가 **움직인 뒤에도** 계약이 Err 를 보고한다.
/// ※(a)의 완결성 신호 = Ok-vs-Err 이지 WriteOutcome.bytes 비교가 아니다(types.rs ~441 — bytes 필드는
///   by-construction 복사값이라 독립 측정이 아님). prefix 물리 배달의 독립 측정은 (b)의 echo 등식이
///   담당한다(자식 stdout 은 transport 와 분리된 관측면).
#[test]
fn physical_pipe_partial_write_then_err_surfaces_as_err() {
    // K = 64KiB 를 읽고 각 청크를 echo 한 뒤 종료. 종료 후 파이프가 끊겨 남은 write_all 이 실패한다.
    //   ★Polish★: 마지막 read 를 남은 (K - total) 로 cap 해 총 소비·echo 를 정확히 K 로 맞춘다
    //   (overshoot 시 "정확히 K 바이트" 등식이 부정확해짐). Read(buf,0,n) 의 n 이 cap.
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
    let transport = Arc::new(transport);

    // pump + 수집 sink(test 1 과 동형) — 자식이 echo 한 prefix 를 seq 순서로 모은다.
    let sink = CollectingSink::new();
    let core = Arc::new(OutputCore::new(Uuid::new_v4(), 0, Arc::new(NoopStatusSink)));
    transport.start(core.clone());
    core.subscribe(Arc::new(sink.clone()));

    // 8MiB 패턴 — K + 파이프 버퍼를 압도. 자식이 K 소비 후 종료 → 파이프 끊김 → in-flight write_all 실패.
    //   ★비반복 스트림★: 고정 시드 xorshift64(seed=0x9E37_79B9_7F4A_7C15 — SplitMix 계열 상수)로 8바이트/
    //   u64 씩 채운다. 결정적(런마다 동일)이고 어떤 오프셋의 K-창도 payload[..K] 와 겹치지 않아, prefix 가
    //   잘못된 오프셋에서 쓰이는 어떤 회귀든 (b) 등식이 잡아낸다. 이 무-앨리어싱 성질은 comment 논증이 아니라
    //   아래 self-check 가 send 전에 기계로 확인한다(패턴이 부적합해지면 send 전에 loud 실패).
    const PAYLOAD_LEN: usize = 8 * 1024 * 1024;
    let payload: Vec<u8> = {
        let mut v = Vec::with_capacity(PAYLOAD_LEN);
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        while v.len() < PAYLOAD_LEN {
            // xorshift64 — 인라인 tiny PRNG.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            v.extend_from_slice(&state.to_le_bytes());
        }
        v.truncate(PAYLOAD_LEN);
        v
    };
    let expected_prefix = payload[..K].to_vec();

    // ★FIX 3★: 모든 실패/타임아웃/spawn-실패 경로는 panic 전 transport.shutdown() 으로 blocked sender·
    //   자식·pump 를 정리한다(다른 테스트 동시 실행 중 누수 방지). `fail!` = shutdown 후 panic 하는 로컬
    //   매크로. self-check·spawn 보다 앞에 정의해 무-앨리어싱 실패와 OS 스레드 생성 실패(Builder::spawn Err)
    //   둘 다 이 경로를 타게 한다.
    macro_rules! fail {
        ($($arg:tt)*) => {{
            transport.shutdown();
            panic!($($arg)*);
        }};
    }

    // ★self-check(기계 검증) — 무-앨리어싱★: 어떤 시프트 s(1..=len-K)에서도 payload[s..s+K] 가 payload[..K]
    //   와 같지 않음을 send 전에 단언한다. 같은 창이 존재하면 그 오프셋에서 잘못 쓰인 회귀를 (b) 등식이
    //   못 잡아(오라클이 그 오프셋에서 눈멂) → 패턴 부적합. early-exit: 대부분 시프트는 첫 바이트부터 달라
    //   payload[s] != payload[0] 만 비교하고 통과. 첫 바이트가 우연히 같은 s 에서만 K 깊이 비교로 내려간다
    //   → debug 빌드에서도 빠르다.
    let p0 = payload[0];
    let self_check_start = Instant::now();
    for s in 1..=(payload.len() - K) {
        if payload[s] == p0 && payload[s..s + K] == expected_prefix[..] {
            fail!("무-앨리어싱 self-check 실패: shift s={s} 에서 payload[s..s+K] == payload[..K] — 이 패턴은 부적합(그 오프셋에서 잘못 쓰인 회귀를 (b) 오라클이 못 잡아 눈멂). 시드/생성기를 바꿔 재생성 필요.");
        }
    }
    let self_check_ms = self_check_start.elapsed().as_millis();
    eprintln!(
        "[self-check] non-aliasing scan of {} shifts: {self_check_ms}ms",
        payload.len() - K
    );

    // ★watchdog(FIX-1 테스트와 동형)★: send 를 별도 스레드에서 돌려 hang(파이프가 안 끊기고
    //   무한 블록)을 데드라인으로 큰 실패로 전환한다. 결과는 채널로 회수.
    // ★thread-creation 실패 대응(round-3 MEDIUM)★: raw spawn 은 OS 스레드 생성 실패 시 shutdown 없이
    //   panic → Builder::spawn 으로 Err 를 받아 fail!(shutdown 후 panic) 로 라우팅. 여기선 아직 send 를
    //   못 냈고 다른 대기 스레드도 없으므로 park 잔여 없음(shutdown 이 자식·pump 만 정리).
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = transport.clone();
    let send_thread = match std::thread::Builder::new().spawn(move || {
        let r = sender.send_input(InputEvent::Raw(payload));
        let _ = tx.send(r);
    }) {
        Ok(h) => h,
        Err(e) => fail!("send 스레드 OS 생성 실패: {e}"),
    };

    let result = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(r) => r,
        Err(_) => {
            fail!("send_input 이 30s 안에 반환하지 않음 — prefix 쓴 뒤 파이프가 안 끊겨 hang(회귀)")
        }
    };
    // send 스레드가 panic 했으면 join 이 Err — 순진한 expect() 는 shutdown 없이 panic 해
    //   all-panic-paths-cleanup 계약을 깬다. fail! 로 라우팅해 자식/pump 를 정리한다.
    if send_thread.join().is_err() {
        fail!("send 스레드가 panic — send_input 내부 실패");
    }

    // 단언 (a): prefix 가 물리적으로 쓰였어도 write_all 계약이 Err 로 표면화 → 부분 배달이 Ok 로
    //   위장되지 않는다.
    if !matches!(result, Err(PtyError::WriteFailed(_))) {
        fail!("prefix 쓴 뒤 파이프 끊김이 WriteFailed 로 표면화돼야 함(Ok 위장 금지): {result:?}");
    }

    // 단언 (b): echo 로 돌아온 prefix 가 정확히 K 바이트 = payload[..K] 여야 한다. K 바이트가 모일
    //   때까지 데드라인 폴링(자식 echo 가 pump 를 거쳐 sink 에 도달하는 데 시간 소요) — 도달 못하면
    //   prefix 미배달 = vacuous pass 의심으로 큰 실패.
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

    transport.shutdown();
    core.join_pump(Duration::from_secs(10));
}
