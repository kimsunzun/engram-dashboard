//! ④ 배달 동사(`AgentSession::submit_input_observed`)의 **물리 경계** 회귀망.
//!
//! 이 파일의 두 항목은 큐 도입이 깨뜨린 두 계약을 각각 잰다. ★둘 다 seam 이 아니라 **실 OS 파이프**를
//! 지난다★ — 재려는 것이 "무엇을 호출했나" 가 아니라 **수신자가 무엇을 언제 읽었나**이기 때문이다.
//!
//! ★왜 seam 으로는 못 재나(기존 항목이 놓친 자리)★: `session.rs` 의 기존 pacing 회귀 항목은 주입한
//!   sleeper 가 **불렸는지**만 기록한다. 큐가 생긴 뒤의 결함은 "대기를 안 했다" 가 아니라 "대기를 **엉뚱한
//!   시점부터** 쟀다" 라서, 호출 기록만 보는 오라클은 그 결함을 그대로 통과시킨다. 실제 read 경계를 봐야
//!   갈린다.
//!
//! ★Windows 전용★: 자식이 powershell 이라 Windows 에서만 컴파일·실행한다(프로젝트 전제).
// ADR-0088 · ADR-0038
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{AgentBackend, InputEncoder, ShellBackend};
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::session::AgentSession;
use engram_dashboard_agent::transport::stdio::StdioTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, OutputFrame, OutputPayload, OutputSink,
    SinkError, SinkId, StatusSink,
};

// ── 수집 sink: 자식이 되울린 텍스트를 순서대로 모은다 ───────────────────────────────────
#[derive(Clone)]
struct TextSink {
    id: SinkId,
    chunks: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl TextSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn text(&self) -> String {
        let mut v = self.chunks.lock().unwrap().clone();
        v.sort_by_key(|(seq, _)| *seq);
        let mut out = Vec::new();
        for (_, b) in v {
            out.extend_from_slice(&b);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}
impl OutputSink for TextSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.chunks.lock().unwrap().push((frame.seq, b.to_vec()));
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
    fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
}

/// 실 파이프 자식 위에 `AgentSession` 을 세운다. ★encoder = `Raw`★ — 그래야 `submit_sequence()` 가
/// `Some(b"\r")` 이라 이 파일이 재려는 **본문/제출 두 write** 경로가 실제로 돈다.
/// ★pacing·sleeper 는 운영 기본값 그대로 둔다★(`with_submit_pacing` 로 줄이지 않는다) — 재려는 것이
/// 그 대기가 만드는 물리 간격이다.
fn session_over(script: &str) -> (AgentSession, TextSink) {
    let spec = CommandSpec {
        program: "powershell.exe".into(),
        args: vec!["-NoProfile".into(), "-Command".into(), script.into()],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = StdioTransport::open(&spec, false, None).expect("open");
    let id = Uuid::new_v4();
    let core = Arc::new(OutputCore::new(
        id,
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    let shell_cmd = engram_dashboard_agent::profile::AgentCommand::Shell {
        program: "powershell.exe".into(),
        args: vec![],
    };
    let backend_caps = ShellBackend.capabilities(&shell_cmd);
    let reads_messages = engram_dashboard_agent::backend::reads_messages(&shell_cmd);
    let session = AgentSession::new(
        id,
        PathBuf::from("."),
        0,
        80,
        24,
        Arc::new(AtomicU8::new(0)),
        backend_caps,
        InputEncoder::Raw,
        reads_messages,
        core,
        transport,
    );
    let sink = TextSink::new();
    session.subscribe(Arc::new(sink.clone()));
    (session, sink)
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// FINDING 2 — 제출 간격은 **본문이 실제로 나간 뒤**부터 재야 한다
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★증명한다★: 제출 바이트(CR)가 **자기만의 read** 로 수신자에게 도착한다 — 즉 본문 꼬리와 한 덩이로
///   묶이지 않는다. 그것이 `backend::SUBMIT_PACING` 이 실측으로 지키려는 성질 그 자체다(간격이 없으면
///   claude TUI 가 CR 을 붙여넣은 텍스트의 일부로 읽어 **턴을 시작하지 않는다** — ADR-0038).
///
/// ★오라클★: 자식이 `Read()` 한 번마다 `READ <바이트수> <마지막바이트>` 한 줄을 되울린다. 마지막 줄이
///   `READ 1 13` 이면 CR 이 홀로 읽힌 것이다. 어떤 줄이든 마지막 바이트가 13 인데 길이가 1 이 아니면
///   본문과 함께 읽힌 것이고 = 이 결함이다.
///
/// ★이 항목이 **고치기 전 코드에서 실제로 실패하는 이유**★: 고치기 전에는 대기를 **큐에 넣은 시각**부터
///   쟀다. 본문 드레인이 대기보다 오래 걸리면(아래 자식은 그렇게 만들어져 있다) 500ms 가 지나도 본문이
///   아직 나가는 중이라, 라이터가 본문을 끝내자마자 CR 을 이어 쓴다 → CR 이 본문 꼬리와 같은 read 에
///   실린다. 고친 뒤에는 본문의 OS 착지를 먼저 확인하고 그 뒤 500ms 를 자므로 파이프가 비어 CR 이 홀로 선다.
///
/// ★★리뷰어의 반례가 옳았다는 것을 이 자식이 보여 준다 — 「자식이 stdin 을 안 읽으니 어차피 안 먹힌다」는
///   변명이 성립하지 않는다★★: 아래 자식은 **쉬지 않고 정상적으로 읽는다.** 그런데도 본문이 파이프
///   버퍼보다 훨씬 커서 드레인에 여러 주기가 걸리고, 그 사이 간격이 0 으로 무너진다.
#[test]
fn submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles() {
    // 8KiB 씩 읽고 매 read 를 보고한 뒤 잠깐 쉰다 — "정상이지만 느린 수신자". 본문(아래 512KiB+)을
    //   드레인하는 데 대략 64 주기가 필요해 SUBMIT_PACING(500ms)보다 오래 걸린다.
    let script = "\
$stdin=[Console]::OpenStandardInput();\
$out=New-Object System.IO.StreamWriter([Console]::OpenStandardOutput());\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){\
$out.WriteLine(\"READ $n $($buf[$n-1])\");$out.Flush();\
Start-Sleep -Milliseconds 10}";

    let (session, sink) = session_over(script);

    // ★8192 의 배수를 피한다★: 딱 떨어지면 본문 마지막 read 가 정확히 경계에서 끝나 CR 이 **우연히**
    //   홀로 실릴 수 있다 — 그러면 결함이 있어도 통과한다(오라클이 눈먼다).
    const BODY: usize = 512 * 1024 + 777;
    let body = vec![b'x'; BODY];

    let started = Instant::now();
    session
        .submit_input_observed(&body)
        .expect("정상 수신자 상대 배달은 성공해야 한다");
    let elapsed = started.elapsed();

    // 제출까지 마친 뒤 자식이 마지막 read 를 보고할 여유.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = sink.text();
        if text.lines().filter(|l| l.starts_with("READ ")).count() > 0
            && text.trim_end().ends_with("13")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "20s 안에 CR(13) 보고가 안 왔다 — 제출 바이트가 도착하지 않았다. 지금까지: {}",
            tail(&text)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let text = sink.text();
    let reads: Vec<(usize, u8)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.strip_prefix("READ ")?.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    assert!(!reads.is_empty(), "자식이 read 를 하나도 보고하지 않았다");

    let total: usize = reads.iter().map(|(n, _)| *n).sum();
    assert_eq!(
        total,
        BODY + 1,
        "수신 총량이 본문+CR 과 다르다 — 유실/중복(읽은 줄: {})",
        tail(&text)
    );

    let (last_n, last_b) = *reads.last().expect("비지 않음");
    assert_eq!(
        (last_n, last_b),
        (1, 13),
        "★제출 바이트가 자기만의 read 로 도착하지 않았다★ — 본문 꼬리와 한 덩이로 묶였다(마지막 read = {last_n} 바이트, 끝 바이트 {last_b}). \
         본문 드레인 {elapsed:?}. 이것이 SUBMIT_PACING 이 막으려는 바로 그 상태다. 읽은 줄: {}",
        tail(&text)
    );

    session.kill(Duration::from_secs(5));
}

/// 실패 메시지가 화면을 덮지 않게 마지막 몇 줄만.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(6);
    lines[start..].join(" | ")
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// FINDING 1 — 배달 영수증은 **수락**이 아니라 **착지**를 뜻해야 한다
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★증명한다★: 본문이 실제로는 못 나갔는데 `submit_input_observed` 가 `Ok` 를 내지 않는다.
///
/// ★왜 이것이 CRITICAL 이었나★: 이 동사의 `Ok` 가 그대로 배달 영수증(`InjectReceipt` →
///   `DeliveryObservation`)이 되고, 원장은 그것을 "배달 성공" 으로 적는다. 수락을 착지로 신고하면
///   ADR-0088 이 가르려고 존재하는 두 경우가 **정확히 뒤집힌다** — 실제로는 전송이 실패했는데 원장에는
///   "배달됐고 모델이 무시했다" 로 남는다. "다음 호출이 `Err` 를 낸다" 는 이미 적힌 그 레코드를 고치지 못한다.
///
/// ★이 항목이 **고치기 전 코드에서 실제로 실패하는 이유**★: 고치기 전 `submit_input_observed` 는 큐가
///   받아 주기만 하면 `Ok(WriteOutcome)` 를 냈다. 아래 자식은 조금 읽고 죽으므로 라이터의 쓰기가 실패하고
///   본문 대부분이 영영 안 나가는데도 영수증이 나왔다.
///
/// ★대조군을 같이 둔다★: 멀쩡한 수신자에게는 그대로 `Ok` 여야 한다 — 없으면 "전부 실패시켜서" 통과하는
///   구현이 이 항목을 만족시킨다.
#[test]
fn receipt_is_refused_when_the_body_never_reaches_the_child() {
    // 대조군 먼저 — 계속 읽는 자식에게는 배달이 성립한다.
    {
        let script = "\
$stdin=[Console]::OpenStandardInput();\
$buf=New-Object byte[] 8192;\
while(($n=$stdin.Read($buf,0,$buf.Length)) -gt 0){}";
        let (session, _sink) = session_over(script);
        session
            .submit_input_observed(b"healthy-recipient")
            .expect("★대조군★ 정상 수신자 상대 배달은 영수증이 나와야 한다");
        session.kill(Duration::from_secs(5));
    }

    // 본 항목 — 64KiB 만 읽고 죽는 자식. 본문은 그보다 훨씬 크다.
    const K: usize = 64 * 1024;
    let script = format!(
        "\
$stdin=[Console]::OpenStandardInput();\
$buf=New-Object byte[] 4096;\
$total=0;\
while($total -lt {K}){{\
$cap=[Math]::Min($buf.Length,{K}-$total);\
$n=$stdin.Read($buf,0,$cap);\
if($n -le 0){{break}};\
$total+=$n}};\
exit 0"
    );
    let (session, _sink) = session_over(&script);

    let result = session.submit_input_observed(&vec![b'y'; 512 * 1024]);
    assert!(
        result.is_err(),
        "★안 나간 본문에 배달 영수증이 나왔다★ — 원장이 '전송 실패' 를 '모델이 무시' 로 뒤집어 적는다: {result:?}"
    );

    session.kill(Duration::from_secs(5));
}
