//! JSONL 레코드 타입 + 직렬화 + raw stream-json 라인 파서 (ADR-0090 Stage 2 파일럿) — 순수.
//!
//! ## 역할
//! (1) **JSONL 레코드**: 파일럿이 런마다 append 하는 한 줄(NDJSON) 레코드들의 serde 타입. `Record` enum
//!     한 종류가 `type` 태그로 갈린다(header/turn/injection/probe/histogram/compaction/summary/stall/compact).
//! (2) **raw stream-json 라인 파서(순수)**: `system/init` 라인의 모델 id 추출·라인의 event 타입 히스토그램
//!     키 산출·"compact" 문자열 포함 여부. **이 함수들은 raw stream-json/트랜스크립트 라인을 받는다** —
//!     `experiment::transcript` 탭이 claude 가 남기는 raw 세션 JSONL 을 읽어 이 파서들로 모델 id·히스토그램·
//!     compact 마커를 추출한다(live 경로).
//!
//! ## ★관측 경로(정직 범위)★
//! 스폰된 json 에이전트의 pump 는 `ClaudeStreamDecoder` 를 거쳐 **디코딩된 OutputEvent 만** OutputSink 로
//! 흘린다(raw 라인은 decoder 내부에서 소비돼 사라진다 — backend/claude.rs). 그래서 `system/init` 모델 id·
//! compact 라인·cache 토큰(실 컨텍스트)은 **스폰 경로에서 직접 못 얻는다**(ADR-0090 제약: 코어 무수정).
//! 우회 = `transcript` 탭: 우리가 통제하는 세션 id(ADR-0008)로 claude 가 디스크에 남기는 raw 트랜스크립트를
//! best-effort 로 읽어 이 파서들로 보강한다. 그래서 아래 raw 파서들은 **live 코드 경로**다(트랜스크립트 탭이
//! 호출). 트랜스크립트가 부재하면(탭 실패) 드라이버는 디코딩 이벤트만으로 폴백하고 모델 id 는 note 로 남긴다.
//!
//! ## 핵심 불변식
//! - **필러 원문 미기록**(ADR-0090 d2): turn 레코드는 body 를 담지 않고 sha256 + len 만.
//! - **응답 4KB 캡**: probe 레코드의 agent 응답은 4KB 로 캡해 파일 폭주 방지.
//! - **summary 항상 기록**(abort 포함): 드라이버가 어떤 종료 경로든 summary 레코드를 마지막에 쓴다.
//!
//! ## 진입점
//! - `Record`(+ 하위 struct): serde. `to_jsonl_line()` = 한 줄 NDJSON.
//! - `sha256_hex(bytes)` / `cap_response(s)`: turn/probe 레코드 조립 헬퍼.
//! - `parse_init_model(line)` / `event_type_key(line)` / `line_mentions_compact(line)`: raw 파서.
// ADR-0090

use serde::{Deserialize, Serialize};

use super::probe::ProbeScores;

/// 한 JSONL 레코드. `type` 태그로 갈린다. 한 런의 파일은 header 1개 + 다수 turn/injection/probe/… +
/// summary 1개(마지막)로 구성된다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    /// 런 헤더 — 재현성 핀 + 설정(ADR-0088 d5a). 파일 첫 줄.
    Header(HeaderRecord),
    /// 한 유저 턴의 관측.
    Turn(TurnRecord),
    /// 주입(inter-agent 메시지 배달) 관측.
    Injection(InjectionRecord),
    /// 지연 프로브 관측 + 채점.
    Probe(ProbeRecord),
    /// event-type 히스토그램(파일럿 산출물 — 디코딩된 이벤트 카탈로그).
    Histogram(HistogramRecord),
    /// suspected-compaction 플래그(외부 근사).
    SuspectedCompaction(SuspectedCompactionRecord),
    /// "compact" 를 언급한 라인 verbatim 캡처(best-effort).
    CompactSignal(CompactSignalRecord),
    /// 턴 대기 타임아웃 등 정지 이벤트.
    Stall(StallRecord),
    /// 런 최종 요약(항상 기록 — abort 포함).
    Summary(SummaryRecord),
}

impl Record {
    /// 한 줄 NDJSON 으로 직렬화(끝에 개행 없음 — 파일 라이터가 붙임). 직렬화 실패는 방어적으로
    /// 에러 문자열 라인으로 대체(레코드 하나 실패가 파일 전체를 깨지 않게).
    pub fn to_jsonl_line(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!(r#"{{"type":"serialize_error","error":"{}"}}"#, e))
    }
}

/// 재현성 핀 + 설정. `resolved_model` 은 stream-json init 에서 얻는 정확 모델 id(못 얻으면 None + note).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderRecord {
    /// claude CLI 버전 문자열(`claude --version` 캡처).
    pub claude_version: Option<String>,
    /// 데몬 git 커밋(best-effort — 못 얻으면 None).
    pub daemon_git_commit: Option<String>,
    /// 요청 모델 핀(예: "sonnet").
    pub model_pin: String,
    /// 트랜스크립트 탭의 system/init 이 보고한 해석된 정확 모델 id(탭 부재/init 라인 부재 시 None).
    pub resolved_model: Option<String>,
    /// resolved_model 을 못 채운 사유(honest gap note — 탭 성공 시 None).
    pub resolved_model_note: Option<String>,
    /// 트랜스크립트 탭이 세션 JSONL 을 찾았나(best-effort 측정 탭 — ADR-0008 경계). false 면 실 usage·모델
    /// id 는 부재하고 드라이버는 문자 추정으로 폴백한다.
    pub transcript_available: bool,
    /// 찾은 트랜스크립트 절대 경로(있으면 — 사후 대조용). 부재 시 None.
    pub transcript_path: Option<String>,
    /// UTC 타임스탬프(RFC3339 근사 문자열).
    pub timestamp_utc: String,
    /// 런 인덱스(0-base).
    pub run_index: u32,
    /// 이 런의 run id(주입 마커 스코프).
    pub run_id: String,
    /// 전 설정 파라미터(재현성 핀) — cli::PilotConfig 를 평면 직렬화한 것.
    pub config: serde_json::Value,
}

/// 한 유저 턴의 관측. filler 본문은 sha256+len 만(원문 미기록 — ADR-0090 d2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub idx: u32,
    /// task|fill|inject|probe|final. (finding 3: 강제 /compact phase 제거로 "compact" kind 폐기. finding 4:
    ///   probe/final 도 이제 TurnRecord 를 남긴다.)
    pub kind: String,
    pub chars_sent: usize,
    /// 보낸 본문의 sha256(hex) — 원문 미기록, 재현 대조용. inject 턴은 authored note body 의 해시(봉투 아님 —
    ///   봉투 실 바이트수는 InjectionRecord.bytes_requested 가 담는다). finding 6: 해시↔chars_sent 정합.
    pub body_sha256: String,
    /// 이 턴의 usage 스냅샷(있으면).
    pub usage: Option<UsageSnapshot>,
    /// 이 턴 처리 벽시계 ms.
    pub wallclock_ms: u64,
}

/// usage 스냅샷 — claude 가 보고한 토큰 + 컨텍스트 크기의 실측/추정 두 계열(캘리브레이션 데이터).
///
/// ★두 계열을 나란히 기록하는 이유(ADR-0090 캘리브레이션)★: 디코딩된 `Usage.input_tokens` 는 그 턴의
/// 증분 입력일 뿐 누적 컨텍스트가 아니다. 실 컨텍스트 크기는 트랜스크립트 탭의 cache 항 합(`RealUsage.
/// context_footprint`)에 있으나 탭이 best-effort 라 부재할 수 있다. 그래서 `context_tokens_real`(탭이
/// 있을 때)과 `context_tokens_estimate`(우리가 보낸 누적 문자수 기반)를 **둘 다** 남겨 사후에 추정↔실측
/// 비율을 캘리브레이션한다.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// 디코딩된 Usage.input_tokens(그 턴 증분 — 누적 컨텍스트 아님).
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 트랜스크립트 탭이 준 실 컨텍스트 footprint(input + cache_creation + cache_read). 탭 부재 시 None.
    pub context_tokens_real: Option<u64>,
    /// 우리가 보낸 누적 문자수 기반 컨텍스트 토큰 추정(항상 있음 — 폴백/캘리브레이션 기준).
    pub context_tokens_estimate: u64,
}

/// 주입 관측(실 control 경로 배달). msg_id·배달 관측(bytes/epoch)을 담는다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionRecord {
    /// 몇 번째 주입인가(0-base).
    pub k: u32,
    /// 이 주입이 발화된 fill 분율(설정 inject_at 값).
    pub at_fraction: f64,
    /// 배달된 논리 메시지 id.
    pub msg_id: String,
    /// codeword(정답 — 프로브 채점 대조용, 이건 실험 메타라 기록).
    pub codeword: String,
    /// 발신자 표시 이름.
    pub sender_name: String,
    /// 배달 성공 여부(DeliveryObservation.is_delivered).
    pub delivered: bool,
    /// 요청 바이트(봉투 크기).
    pub bytes_requested: usize,
    /// 실제 쓰인 바이트(성공 시 Some).
    pub bytes_written: Option<usize>,
    /// write 가 착지한 수신자 epoch(ADR-0088 — 성공 시 Some).
    pub to_epoch: Option<u32>,
    /// write 에러(실패 시 Some).
    pub error: Option<String>,
}

/// 지연 프로브 관측 + 채점. 질문 + 응답(4KB 캡) + 불리언 점수.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRecord {
    /// 어느 주입에 대한 프로브인가(FINAL REPORT 는 None).
    pub for_injection_k: Option<u32>,
    /// ★finding 4★: 이 프로브가 소비한 실 턴 인덱스(0-base). 이전엔 프로브가 turn_idx 를 올리면서도
    ///   TurnRecord/turn_idx 를 안 남겨 인덱스 구멍(hole)이 생겼다 — 이제 프로브도 자기 turn_idx 를 실어
    ///   TurnRecord(kind="probe") 와 정렬된다. send_and_collect 가 부여한 그 턴의 인덱스.
    pub turn_idx: u32,
    /// ★finding 4★: 이 프로브 턴의 usage 스냅샷(트랜스크립트 탭 있으면 실측 + 추정, 없으면 추정만). 프로브도
    ///   컨텍스트를 소비하는 실 턴이므로 fill/inject 턴과 동일하게 계열에 남긴다(사후 캘리브레이션 정합).
    pub usage: Option<UsageSnapshot>,
    /// 프로브 질문 텍스트.
    pub question: String,
    /// 에이전트 응답(4KB 캡).
    pub response: String,
    /// 이 프로브가 FINAL REPORT 였나.
    pub final_report: bool,
    pub scores: ProbeScores,
}

/// event-type 히스토그램(파일럿 산출물). key=이벤트 타입/서브타입, value=건수.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramRecord {
    pub counts: std::collections::BTreeMap<String, u64>,
    /// 히스토그램이 raw stream-json 타입인지 디코딩된 이벤트 variant 인지 명시(honest scope).
    pub source: String,
}

/// suspected-compaction 플래그(외부 근사).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspectedCompactionRecord {
    /// 급감이 관측된 턴 인덱스들.
    pub flagged_turn_idxs: Vec<u32>,
}

/// "compact" 를 언급한 라인 verbatim(best-effort — decoder 를 통과한 Structured/Error 텍스트에서 스캔).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactSignalRecord {
    /// 캡처된 원문(4KB 캡).
    pub verbatim: String,
    /// 어디서 스캔했나(예: "decoded_structured", "decoded_error").
    pub source: String,
}

/// 정지 이벤트(턴 대기 타임아웃 등).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StallRecord {
    pub turn_idx: u32,
    pub reason: String,
    pub waited_ms: u64,
}

/// 런 최종 요약(항상 기록). abort 사유가 있으면 Some.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryRecord {
    pub max_context_tokens: u64,
    pub total_turns: u32,
    pub duration_ms: u64,
    /// abort 로 끝났으면 사유(정상 완료면 None).
    pub abort_reason: Option<String>,
    /// ★finding 1★: 런 끝 authoritative 트랜스크립트 파싱이 확정한 정확 모델 id(있으면). 헤더의
    ///   resolved_model 은 스폰 직후엔 대개 None(트랜스크립트가 첫 턴 뒤에야 써짐)이라 재현성 핀(ADR-0088
    ///   d5a)이 헤더만 보면 유실된다 — summary 는 런 끝 시점의 확정 모델 id 를 실어 핀을 보존한다. 탭
    ///   부재/모델 라인 부재 시 None.
    pub resolved_model: Option<String>,
    /// ★finding 1★: 런 끝 시점 트랜스크립트 탭 확보 여부(best-effort 측정 탭 — ADR-0008 경계). false 면
    ///   실 usage·모델 id 는 부재하고 드라이버가 문자 추정으로만 진행했다는 뜻.
    pub transcript_available: bool,
    /// ★finding 1★: 확보한 트랜스크립트 절대 경로(있으면 — 사후 대조용). 부재 시 None.
    pub transcript_path: Option<String>,
}

/// 응답 텍스트 4KB 캡(ADR-0090 — probe/compact verbatim 폭주 방지). 바이트 경계로 자르되 UTF-8
/// 문자 경계를 존중한다(멀티바이트 중간 절단 방지 — lossy 아님, 안전 경계까지만).
pub const RESPONSE_CAP_BYTES: usize = 4096;

/// s 를 최대 RESPONSE_CAP_BYTES 바이트로 캡(UTF-8 문자 경계 존중).
pub fn cap_response(s: &str) -> String {
    if s.len() <= RESPONSE_CAP_BYTES {
        return s.to_string();
    }
    // RESPONSE_CAP_BYTES 이하이면서 문자 경계인 최대 위치를 찾는다.
    let mut end = RESPONSE_CAP_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 바이트열의 sha256 을 hex 문자열로. **외부 crate 없이** std 만으로 구현(ADR-0090 no-new-deps).
/// 필러 원문 대신 이 해시를 turn 레코드에 실어 재현 대조를 가능케 한다(원문 미기록 불변식).
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// stream-json/트랜스크립트 `system`/init 라인에서 해석된 모델 id 를 뽑는다(raw 라인 파서). 라인이
/// 비-JSON 이거나 system/init 이 아니거나 model 필드가 없으면 None. (transcript 탭이 라인마다 호출 —
/// 모듈 헤더 ★관측 경로★ 참조.)
///
/// 실측 스키마(claude 2.1.170): `{"type":"system","subtype":"init",...,"model":"claude-...",...}`.
pub fn parse_init_model(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("system") {
        return None;
    }
    // ★finding 14 fix★: subtype 이 **반드시 "init" 이어야** authoritative 모델 id 로 인정한다. 이전엔
    //   subtype 부재 시 model 존재만으로 통과해 `{"type":"system","model":"spoof"}` 같은 위조 라인을
    //   권위 있는 모델 id 로 받아들였다(스푸핑 표면). init 서브타입이 아닌 system 라인은 거부한다 —
    //   stream-json headless 경로의 진짜 모델 id 는 model_from_assistant_line(assistant.message.model)이
    //   담당하므로 이 강화로 잃는 정당 소스는 없다.
    if v.get("subtype").and_then(|s| s.as_str()) != Some("init") {
        return None;
    }
    v.get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

/// raw stream-json/트랜스크립트 라인의 히스토그램 키 — `type` 또는 `type/subtype`. 비-JSON 이면
/// "non_json". (파일럿 event 카탈로그 산출용 — transcript 탭이 라인마다 호출.)
pub fn event_type_key(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return "empty".to_string();
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => {
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("no_type");
            match v.get("subtype").and_then(|s| s.as_str()) {
                Some(sub) => format!("{ty}/{sub}"),
                None => ty.to_string(),
            }
        }
        Err(_) => "non_json".to_string(),
    }
}

/// 라인의 type/subtype 중 하나라도 "compact" 를 포함하나(대소문자 무시). compact 관련 라인 verbatim
/// 캡처 트리거(ADR-0090 §5).
pub fn line_mentions_compact(line: &str) -> bool {
    let v: serde_json::Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(_) => return line.to_ascii_lowercase().contains("compact"),
    };
    let has = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.to_ascii_lowercase().contains("compact"))
            .unwrap_or(false)
    };
    has("type") || has("subtype")
}

// ── std-only SHA-256 (FIPS 180-4) ──────────────────────────────────────────────────
// ★왜 손구현인가★: ADR-0090 이 신규 의존성 추가를 금지한다(no new deps). 워크스페이스에 sha2 가 없어
//   필러 본문 해시(원문 미기록 불변식의 대체)를 위해 표준 SHA-256 을 std 만으로 구현한다. 성능은
//   무관하다(런당 수십~수백 문서, 벽시계에 안 잡힘). 정확성은 아래 단위테스트가 표준 test vector로 확인.

struct Sha256;

impl Sha256 {
    fn digest(data: &[u8]) -> [u8; 32] {
        // 초기 해시값(FIPS 180-4 §5.3.3).
        let mut h: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        // 라운드 상수(§4.2.2).
        const K: [u32; 64] = [
            0x428a_2f98,
            0x7137_4491,
            0xb5c0_fbcf,
            0xe9b5_dba5,
            0x3956_c25b,
            0x59f1_11f1,
            0x923f_82a4,
            0xab1c_5ed5,
            0xd807_aa98,
            0x1283_5b01,
            0x2431_85be,
            0x550c_7dc3,
            0x72be_5d74,
            0x80de_b1fe,
            0x9bdc_06a7,
            0xc19b_f174,
            0xe49b_69c1,
            0xefbe_4786,
            0x0fc1_9dc6,
            0x240c_a1cc,
            0x2de9_2c6f,
            0x4a74_84aa,
            0x5cb0_a9dc,
            0x76f9_88da,
            0x983e_5152,
            0xa831_c66d,
            0xb003_27c8,
            0xbf59_7fc7,
            0xc6e0_0bf3,
            0xd5a7_9147,
            0x06ca_6351,
            0x1429_2967,
            0x27b7_0a85,
            0x2e1b_2138,
            0x4d2c_6dfc,
            0x5338_0d13,
            0x650a_7354,
            0x766a_0abb,
            0x81c2_c92e,
            0x9272_2c85,
            0xa2bf_e8a1,
            0xa81a_664b,
            0xc24b_8b70,
            0xc76c_51a3,
            0xd192_e819,
            0xd699_0624,
            0xf40e_3585,
            0x106a_a070,
            0x19a4_c116,
            0x1e37_6c08,
            0x2748_774c,
            0x34b0_bcb5,
            0x391c_0cb3,
            0x4ed8_aa4a,
            0x5b9c_ca4f,
            0x682e_6ff3,
            0x748f_82ee,
            0x78a5_636f,
            0x84c8_7814,
            0x8cc7_0208,
            0x90be_fffa,
            0xa450_6ceb,
            0xbef9_a3f7,
            0xc671_78f2,
        ];

        // 패딩(§5.1.1): 0x80 + 0x00... + 64-bit 비트길이(big-endian).
        let bit_len = (data.len() as u64).wrapping_mul(8);
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0x00);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (i, word) in w.iter_mut().enumerate().take(16) {
                let j = i * 4;
                *word = u32::from_be_bytes([chunk[j], chunk[j + 1], chunk[j + 2], chunk[j + 3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            let mut f = h[5];
            let mut g = h[6];
            let mut hh = h[7];

            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }

            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }

        let mut out = [0u8; 32];
        for (i, word) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        // FIPS 표준 test vector.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn sha256_multiblock() {
        // 64 바이트 초과(멀티블록) 벡터 — 448 비트 경계 패딩 회귀.
        let input = "a".repeat(1000);
        assert_eq!(
            sha256_hex(input.as_bytes()),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn cap_response_leaves_short_untouched() {
        assert_eq!(cap_response("hello"), "hello");
    }

    #[test]
    fn cap_response_caps_long() {
        let long = "x".repeat(RESPONSE_CAP_BYTES + 500);
        let capped = cap_response(&long);
        assert_eq!(capped.len(), RESPONSE_CAP_BYTES);
    }

    #[test]
    fn cap_response_respects_char_boundary() {
        // 멀티바이트 문자가 캡 경계에 걸리면 그 앞에서 자른다(중간 절단 X).
        let s = "가".repeat(RESPONSE_CAP_BYTES); // '가' = 3 bytes.
        let capped = cap_response(&s);
        assert!(capped.len() <= RESPONSE_CAP_BYTES);
        // 유효 UTF-8 이어야(경계 존중).
        assert!(std::str::from_utf8(capped.as_bytes()).is_ok());
    }

    #[test]
    fn parse_init_model_extracts_model() {
        let line =
            r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-5-20250929","cwd":"/x"}"#;
        assert_eq!(
            parse_init_model(line).as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
    }

    #[test]
    fn parse_init_model_rejects_non_init() {
        assert_eq!(parse_init_model(r#"{"type":"assistant"}"#), None);
        assert_eq!(
            parse_init_model(r#"{"type":"system","subtype":"other","model":"x"}"#),
            None
        );
        assert_eq!(parse_init_model("not json"), None);
    }

    #[test]
    fn parse_init_model_rejects_system_without_subtype() {
        // ★finding 14 regression★: subtype 없는 system 라인은 model 이 있어도 거부(스푸핑 차단).
        assert_eq!(
            parse_init_model(r#"{"type":"system","model":"spoof"}"#),
            None,
            "subtype 없는 system 라인은 authoritative 모델 id 아님(스푸핑 방지)"
        );
        // subtype 이 init 이 아니어도 거부.
        assert_eq!(
            parse_init_model(r#"{"type":"system","subtype":"result","model":"spoof"}"#),
            None
        );
    }

    #[test]
    fn event_type_key_forms() {
        assert_eq!(event_type_key(r#"{"type":"assistant"}"#), "assistant");
        assert_eq!(
            event_type_key(r#"{"type":"system","subtype":"init"}"#),
            "system/init"
        );
        assert_eq!(event_type_key("garbage"), "non_json");
        assert_eq!(event_type_key(""), "empty");
        assert_eq!(event_type_key(r#"{"foo":1}"#), "no_type");
    }

    #[test]
    fn line_mentions_compact_detects() {
        assert!(line_mentions_compact(
            r#"{"type":"system","subtype":"compact_boundary"}"#
        ));
        assert!(line_mentions_compact(r#"{"type":"compacting"}"#));
        assert!(!line_mentions_compact(r#"{"type":"assistant"}"#));
        // 비-JSON 은 문자열 스캔 fallback.
        assert!(line_mentions_compact("some compact log line"));
    }

    #[test]
    fn record_roundtrips_jsonl() {
        let rec = Record::Turn(TurnRecord {
            idx: 3,
            kind: "fill".to_string(),
            chars_sent: 12000,
            body_sha256: sha256_hex(b"body"),
            usage: Some(UsageSnapshot {
                input_tokens: 5000,
                output_tokens: 12,
                context_tokens_real: Some(29288),
                context_tokens_estimate: 5000,
            }),
            wallclock_ms: 4200,
        });
        let line = rec.to_jsonl_line();
        // 한 줄이어야(NDJSON) — 내부 개행 없음.
        assert!(!line.contains('\n'));
        // type 태그 존재.
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "turn");
        assert_eq!(v["kind"], "fill");
        assert_eq!(v["usage"]["input_tokens"], 5000);
        assert_eq!(v["usage"]["context_tokens_real"], 29288);
        assert_eq!(v["usage"]["context_tokens_estimate"], 5000);
    }

    #[test]
    fn summary_record_serializes_abort() {
        let rec = Record::Summary(SummaryRecord {
            max_context_tokens: 42000,
            total_turns: 15,
            duration_ms: 90000,
            abort_reason: Some("turn timeout".to_string()),
            resolved_model: None,
            transcript_available: false,
            transcript_path: None,
        });
        let v: serde_json::Value = serde_json::from_str(&rec.to_jsonl_line()).unwrap();
        assert_eq!(v["type"], "summary");
        assert_eq!(v["abort_reason"], "turn timeout");
    }

    #[test]
    fn summary_record_carries_resolved_model() {
        // ★finding 1★: summary 는 런 끝 authoritative 모델 id 를 실어 재현성 핀(ADR-0088 d5a)을 보존한다.
        let rec = Record::Summary(SummaryRecord {
            max_context_tokens: 49000,
            total_turns: 22,
            duration_ms: 120000,
            abort_reason: None,
            resolved_model: Some("claude-sonnet-4-6".to_string()),
            transcript_available: true,
            transcript_path: Some("/x/abc.jsonl".to_string()),
        });
        let v: serde_json::Value = serde_json::from_str(&rec.to_jsonl_line()).unwrap();
        assert_eq!(v["type"], "summary");
        assert_eq!(v["resolved_model"], "claude-sonnet-4-6");
        assert_eq!(v["transcript_available"], true);
        assert_eq!(v["transcript_path"], "/x/abc.jsonl");
    }

    #[test]
    fn probe_record_carries_turn_idx_and_usage() {
        // ★finding 4★: 프로브 레코드는 자기 turn_idx 와 usage 스냅샷을 실어 TurnRecord(kind="probe")와 정렬된다.
        let rec = Record::Probe(ProbeRecord {
            for_injection_k: Some(0),
            turn_idx: 7,
            usage: Some(UsageSnapshot {
                input_tokens: 3,
                output_tokens: 40,
                context_tokens_real: Some(41000),
                context_tokens_estimate: 12000,
            }),
            question: "q".to_string(),
            response: "a".to_string(),
            final_report: false,
            scores: ProbeScores {
                sender_recalled: true,
                id_prefix_recalled: false,
                codeword_recalled: true,
                final_count_correct: false,
                doc1_title_recalled: false,
            },
        });
        let v: serde_json::Value = serde_json::from_str(&rec.to_jsonl_line()).unwrap();
        assert_eq!(v["type"], "probe");
        assert_eq!(v["turn_idx"], 7);
        assert_eq!(v["usage"]["context_tokens_real"], 41000);
    }
}
