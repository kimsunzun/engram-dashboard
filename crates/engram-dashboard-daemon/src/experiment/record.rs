//! JSONL record types + serialization + raw stream-json line parser (ADR-0090 Stage 2 pilot).
//!
//! ## Role
//! (1) **JSONL records**: serde types of the NDJSON records the pilot appends on every run.
//! (2) **raw stream-json line parser**: the input is a raw stream-json/transcript line (not a decoded OutputEvent).
//!     It is the live path the `experiment::transcript` tap calls on every line — why raw lines live only in the tap is
//!     in the `transcript` module header, the authority.
//!
//! ## Core invariants
//! - **Filler original text not recorded** (ADR-0090 d2): a turn record carries no body, only sha256 + len.
// ADR-0090

use engram_dashboard_base::text::hex_lower;
use serde::{Deserialize, Serialize};

use super::probe::ProbeScores;

/// One run's file = 1 header + many turn/injection/probe/… + 1 summary (last).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Header(HeaderRecord),
    Turn(TurnRecord),
    Injection(InjectionRecord),
    Probe(ProbeRecord),
    Histogram(HistogramRecord),
    SuspectedCompaction(SuspectedCompactionRecord),
    CompactSignal(CompactSignalRecord),
    Stall(StallRecord),
    Summary(SummaryRecord),
}

impl Record {
    /// No newline at the end — the file writer adds it. A serialization failure is replaced by an error line — so one record
    /// does not break the whole file.
    pub fn to_jsonl_line(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!(r#"{{"type":"serialize_error","error":"{}"}}"#, e))
    }
}

/// Reproducibility pin + config (ADR-0088 d5a). First line of the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderRecord {
    pub claude_version: Option<String>,
    pub daemon_git_commit: Option<String>,
    /// Requested model pin (CLI alias — e.g. "sonnet").
    pub model_pin: String,
    /// The resolved exact model id reported by the transcript tap's system/init (None when the tap or the init line is absent).
    pub resolved_model: Option<String>,
    /// Reason resolved_model could not be filled (honest gap note — None when the tap succeeded).
    pub resolved_model_note: Option<String>,
    /// Did the transcript tap find the session JSONL (best-effort measurement tap — ADR-0008 boundary). If false, real usage and model
    /// id are absent and the driver falls back to character estimates.
    pub transcript_available: bool,
    /// Absolute path of the transcript found (for after-the-fact cross-checking). None if absent.
    pub transcript_path: Option<String>,
    /// Approximate RFC3339 string.
    pub timestamp_utc: String,
    /// 0-base.
    pub run_index: u32,
    /// Scope of the injection marker.
    pub run_id: String,
    /// `cli::PilotConfig` serialized flat.
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub idx: u32,
    /// task|fill|inject|probe|final.
    pub kind: String,
    pub chars_sent: usize,
    /// sha256 (hex) of the body sent — the same body as `chars_sent`. For an inject turn it is the hash of the authored note body,
    ///   not the envelope (envelope actual byte count = `InjectionRecord.bytes_requested`).
    pub body_sha256: String,
    pub usage: Option<UsageSnapshot>,
    pub wallclock_ms: u64,
}

/// Keep **both** the measured and estimated series, so the estimated↔measured ratio can be calibrated afterwards (a pilot product).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// Decoded `Usage.input_tokens` — that turn's increment, not the cumulative context.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Real context footprint given by the transcript tap (`RealUsage::context_footprint`). None when the tap is absent.
    pub context_tokens_real: Option<u64>,
    /// An estimate based on the cumulative character count we sent — the fallback and calibration reference.
    pub context_tokens_estimate: u64,
}

/// Injection observation — delivery over the real control path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionRecord {
    /// Which injection is it (0-based).
    pub k: u32,
    /// The fill fraction at which this injection fired (the config inject_at value).
    pub at_fraction: f64,
    pub msg_id: String,
    /// Answers — for cross-checking probe grading. They are experiment meta so they are recorded as an exception to the no-original-text invariant.
    pub codeword: String,
    pub sender_name: String,
    /// `DeliveryObservation::is_delivered`.
    pub delivered: bool,
    /// Envelope size.
    pub bytes_requested: usize,
    pub bytes_written: Option<usize>,
    /// The recipient epoch the write landed on (ADR-0088 — Some on success).
    pub to_epoch: Option<u32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRecord {
    /// None for FINAL REPORT.
    pub for_injection_k: Option<u32>,
    /// ★finding 4★: the real turn index (0-based) this probe consumed — the same index as `TurnRecord` (kind="probe").
    ///   Previously a probe bumped turn_idx without leaving a TurnRecord, creating a hole in the index.
    pub turn_idx: u32,
    /// A probe is also a real turn that consumes context, so it is left in the series the same as fill/inject turns (calibration consistency).
    pub usage: Option<UsageSnapshot>,
    pub question: String,
    /// 4KB cap — may be truncated.
    pub response: String,
    pub final_report: bool,
    pub scores: ProbeScores,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramRecord {
    /// key = event `type` or `type/subtype`, value = count.
    pub counts: std::collections::BTreeMap<String, u64>,
    /// Whether this histogram is of raw stream-json types or of decoded event variants (honest scope).
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspectedCompactionRecord {
    pub flagged_turn_idxs: Vec<u32>,
}

/// Lines mentioning "compact", verbatim (best-effort).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactSignalRecord {
    /// 4KB cap — may be truncated.
    pub verbatim: String,
    /// Where it was scanned — decoded Structured/Error text or a raw transcript line.
    pub source: String,
}

/// Stop event — `reason` is a turn-wait timeout etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StallRecord {
    pub turn_idx: u32,
    pub reason: String,
    pub waited_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryRecord {
    pub max_context_tokens: u64,
    pub total_turns: u32,
    pub duration_ms: u64,
    /// The reason if it ended by abort (None if it completed normally).
    pub abort_reason: Option<String>,
    /// ★finding 1★: the exact model id fixed by the authoritative transcript parse at the end of the run. The header's resolved_model is
    ///   usually None right after spawn (the transcript is written only after the first turn), so the reproducibility pin (ADR-0088 d5a) is
    ///   lost if it relies on the header alone — the summary preserves the pin with the value fixed at the end of the run. None if the tap or the model line is absent.
    pub resolved_model: Option<String>,
    /// Values at the end of the run (the meaning is that of the same-named field in `HeaderRecord`).
    pub transcript_available: bool,
    pub transcript_path: Option<String>,
}

/// Prevents file runaway from probe/compact verbatim (ADR-0090 — suppresses file size and noise).
pub const RESPONSE_CAP_BYTES: usize = 4096;

pub fn cap_response(s: &str) -> String {
    if s.len() <= RESPONSE_CAP_BYTES {
        return s.to_string();
    }
    s[..s.floor_char_boundary(RESPONSE_CAP_BYTES)].to_string()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_lower(&Sha256::digest(data))
}

/// Measured schema (claude 2.1.170): `{"type":"system","subtype":"init",...,"model":"claude-...",...}`.
pub fn parse_init_model(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("system") {
        return None;
    }
    // ★finding 14 fix★: back when the check passed on the mere presence of model without subtype, a forged line like `{"type":"system","model":"spoof"}`
    //   got in as an authoritative model id (a spoofing surface) — do not relax. The real model id of the stream-json headless path is
    //   handled by `model_from_assistant_line` (assistant.message.model), so no legitimate source is lost.
    if v.get("subtype").and_then(|s| s.as_str()) != Some("init") {
        return None;
    }
    v.get("model")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

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
// ★Why hand-implemented★: ADR-0090 forbids adding new dependencies (no new deps) and the workspace has no sha2.
//   Performance is irrelevant — tens to hundreds of documents per run do not register on the wall clock.

struct Sha256;

impl Sha256 {
    fn digest(data: &[u8]) -> [u8; 32] {
        // Initial hash values (FIPS 180-4 §5.3.3).
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
        // Round constants (§4.2.2).
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

        // Padding (§5.1.1).
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
        let s = "가".repeat(RESPONSE_CAP_BYTES); // '가' = 3 bytes.
        let capped = cap_response(&s);
        assert!(capped.len() <= RESPONSE_CAP_BYTES);
        assert!(std::str::from_utf8(capped.as_bytes()).is_ok());
    }

    /// If the cap falls in the middle of a character, cut at the **lower** boundary — keep as much as possible without exceeding it.
    /// 4096 is not a multiple of 3, so in a run of `'가'` (3 bytes) the cap falls in the middle of a character.
    #[test]
    fn cap_response_cuts_at_the_last_boundary_below_the_cap() {
        let s = "가".repeat(RESPONSE_CAP_BYTES);
        let capped = cap_response(&s);
        assert_eq!(capped.len(), RESPONSE_CAP_BYTES / 3 * 3);
        assert!(s.starts_with(&capped));
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
        assert_eq!(
            parse_init_model(r#"{"type":"system","model":"spoof"}"#),
            None,
            "subtype 없는 system 라인은 authoritative 모델 id 아님(스푸핑 방지)"
        );
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
        assert!(!line.contains('\n'));
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
