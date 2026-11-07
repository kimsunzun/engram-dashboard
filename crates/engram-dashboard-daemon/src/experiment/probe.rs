//! Probe grading + suspected-compaction detection (ADR-0090 Stage 2 pilot).
//!
//! ## Core invariants
//! - **Pure and deterministic** — everything is a function of the input alone (0 external state or clock).
//! - **Decision = recall holds even after a delay** (ADR-0088) — an immediate ack is not success. `ProbeScores` is that encoding.

use serde::{Deserialize, Serialize};

/// Grading result of one probe response. Carried as is in the JSONL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeScores {
    pub sender_recalled: bool,
    /// An axis relaxed to prefix recall because the full uuid is long.
    pub id_prefix_recalled: bool,
    pub codeword_recalled: bool,
    /// FINAL REPORT probe only — otherwise always false.
    pub final_count_correct: bool,
    /// DOC-1 title. FINAL REPORT probe only — otherwise always false.
    pub doc1_title_recalled: bool,
}

/// Length of the msg_id prefix-recall criterion (leading bytes).
pub const ID_PREFIX_LEN: usize = 8;

/// Reduces false negatives caused by punctuation and case differences.
fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect()
}

/// Prevents a short token from accidentally matching as part of another word (e.g. so "id" is not caught in "identity").
fn contains_token_seq(haystack_norm: &str, needle: &str) -> bool {
    let needle_norm = normalize(needle);
    let needle_tokens: Vec<&str> = needle_norm.split_whitespace().collect();
    if needle_tokens.is_empty() {
        return false;
    }
    let hay_tokens: Vec<&str> = haystack_norm.split_whitespace().collect();
    hay_tokens
        .windows(needle_tokens.len())
        .any(|w| w == needle_tokens.as_slice())
}

/// ★Why pattern matching (finding 11 fix)★: a plain "is the correct number token in the response" gives a false positive in a sentence like "I received 41 documents;
///   DOC-42 was not received" (correct answer 42), where 42 appears as a document **reference** (DOC-42).
///   So we require an adjacent (±1) context where the correct number reads as a count report.
fn count_reported_with_cue(haystack_norm: &str, expected_count: u32) -> bool {
    // Why the count nouns are here: "Total documents: 42" becomes "total documents 42" after normalization, so the count
    //   noun stands **before** the number.
    const LEADING_CUES: &[&str] = &[
        "total",
        "received",
        "count",
        "counted",
        "number",
        "seen",
        "tally",
        "of",
        "documents",
        "docs",
    ];
    // "document" (singular) also appears in DOC-N references, but when it stands **after** the number it is the count-noun usage so it is included.
    const TRAILING_NOUNS: &[&str] = &["documents", "docs", "document"];
    // When it stands **before** the number it is a document reference (DOC-N) and not a count — excluded.
    const REFERENCE_LABELS: &[&str] = &["doc", "document"];

    let expected = expected_count.to_string();
    let tokens: Vec<&str> = haystack_norm.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if *tok != expected {
            continue;
        }
        let prev = i.checked_sub(1).map(|j| tokens[j]);
        let next = tokens.get(i + 1).copied();
        if prev.map(|p| REFERENCE_LABELS.contains(&p)).unwrap_or(false) {
            continue;
        }
        let leading_ok = prev.map(|p| LEADING_CUES.contains(&p)).unwrap_or(false);
        let trailing_ok = next.map(|n| TRAILING_NOUNS.contains(&n)).unwrap_or(false);
        if leading_ok || trailing_ok {
            return true;
        }
    }
    false
}

/// Only `response` is agent output and `sender_name`·`msg_id`·`codeword`·`expected_*` are all **answers**.
/// `expected_doc_count`/`expected_doc1_title` are ignored if `final_report` is false.
#[allow(clippy::too_many_arguments)]
pub fn score_probe(
    response: &str,
    sender_name: &str,
    msg_id: &str,
    codeword: &str,
    final_report: bool,
    expected_doc_count: u32,
    expected_doc1_title: &str,
) -> ProbeScores {
    let norm = normalize(response);

    let sender_recalled = !sender_name.is_empty() && contains_token_seq(&norm, sender_name);
    let codeword_recalled = !codeword.is_empty() && contains_token_seq(&norm, codeword);

    // ★finding 12 fix★: a raw substring (`response.contains(prefix)`) gave a false positive when the 8-hex prefix landed **inside** an unrelated word
    //   (`deadbeef` in `undeadbeefed`) — do not relax. At the same time, since it is starts_with and not equality,
    //   prefix recall holds even if the agent writes the full id (`1a2b3c4d5e`).
    let id_prefix_recalled = if msg_id.len() >= ID_PREFIX_LEN {
        let prefix = normalize(&msg_id[..ID_PREFIX_LEN]);
        let prefix = prefix.trim();
        if prefix.is_empty() {
            false
        } else {
            norm.split_whitespace().any(|tok| tok.starts_with(prefix))
        }
    } else {
        false
    };

    let (final_count_correct, doc1_title_recalled) = if final_report {
        let count_ok = count_reported_with_cue(&norm, expected_doc_count);
        let title_ok =
            !expected_doc1_title.is_empty() && contains_token_seq(&norm, expected_doc1_title);
        (count_ok, title_ok)
    } else {
        (false, false)
    };

    ProbeScores {
        sender_recalled,
        id_prefix_recalled,
        codeword_recalled,
        final_count_correct,
        doc1_title_recalled,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageSample {
    /// 0-base.
    pub turn_idx: u32,
    /// The context tokens of that turn reported by claude usage (ground truth).
    pub context_tokens: u64,
    /// Did the harness deliberately reset the context just before this turn — if so, the sharp drop is not suspicious.
    pub harness_reset: bool,
}

/// Suspicious if the context tokens between two consecutive turns fall by **more than** this ratio.
pub const COMPACTION_DROP_THRESHOLD: f64 = 0.30;

pub fn real_series_from_footprints(footprints: &[u64]) -> Vec<UsageSample> {
    footprints
        .iter()
        .enumerate()
        .map(|(i, &tokens)| UsageSample {
            turn_idx: i as u32,
            context_tokens: tokens,
            harness_reset: false,
        })
        .collect()
}

/// ★finding 2 — single-source selection contract★: choose **one source** series to run detection on. **Never join
///   two series.**
///
/// ★Why (load-bearing)★: with a mixed series, at the **moment the transcript tap first attaches**, that turn's value dropped sharply as the source switched from the previous turn's (larger)
///   estimate to the (smaller) real first footprint, and that artificial drop crossed the threshold and a fake
///   compaction flag was raised. With a single source that switch itself does not exist — a real drop is still caught. When this guarantee
///   existed only in a `match` inside the bin, it existed only as prose and unit tests could not reach it.
pub fn select_detection_series(
    real_footprints: Option<&[u64]>,
    estimate: &[UsageSample],
) -> Vec<UsageSample> {
    match real_footprints {
        Some(fp) if !fp.is_empty() => real_series_from_footprints(fp),
        _ => estimate.to_vec(),
    }
}

/// Returned element = **the turn_idx of the (later) turn where the drop was observed** (cur in prev→cur).
///
/// ★Why an external approximation★: claude stream-json may not give compaction as an explicit signal (ADR-0090 context),
///   so if context tokens drop sharply without a reset we **suspect** internal compaction. It is a flag, not confirmation —
///   material for the pilot to cross-check against the real signal (if any).
pub fn detect_suspected_compaction(samples: &[UsageSample]) -> Vec<u32> {
    let mut flags = Vec::new();
    for pair in samples.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        if cur.harness_reset {
            continue;
        }
        // If prev is 0 the ratio is meaningless — skip.
        if prev.context_tokens == 0 {
            continue;
        }
        if cur.context_tokens < prev.context_tokens {
            let drop = prev.context_tokens - cur.context_tokens;
            let ratio = drop as f64 / prev.context_tokens as f64;
            if ratio > COMPACTION_DROP_THRESHOLD {
                flags.push(cur.turn_idx);
            }
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(resp: &str, sender: &str, id: &str, code: &str) -> ProbeScores {
        score_probe(resp, sender, id, code, false, 0, "")
    }

    #[test]
    fn scores_full_recall() {
        let s = score(
            "The message came from alpha-agent, id 1a2b3c4d5e, codeword MOONLIGHT.",
            "alpha-agent",
            "1a2b3c4d5e6f7g8h",
            "MOONLIGHT",
        );
        assert!(s.sender_recalled, "발신자 회상");
        assert!(s.id_prefix_recalled, "id 접두 회상");
        assert!(s.codeword_recalled, "codeword 회상");
    }

    #[test]
    fn scores_case_insensitive() {
        let s = score(
            "sender was ALPHA, codeword moonlight",
            "alpha",
            "zzzzzzzzzzzz",
            "MOONLIGHT",
        );
        assert!(s.sender_recalled);
        assert!(s.codeword_recalled);
    }

    #[test]
    fn scores_miss_when_absent() {
        let s = score(
            "I don't remember any message.",
            "alpha",
            "1a2b3c4d",
            "MOONLIGHT",
        );
        assert!(!s.sender_recalled);
        assert!(!s.id_prefix_recalled);
        assert!(!s.codeword_recalled);
    }

    #[test]
    fn id_prefix_requires_min_len() {
        let s = score("id: abc", "x", "abc", "c");
        assert!(!s.id_prefix_recalled);
    }

    #[test]
    fn id_prefix_matches_leading_eight() {
        let s = score(
            "the id started with 1a2b3c4d somewhere",
            "x",
            "1a2b3c4d9999",
            "c",
        );
        assert!(s.id_prefix_recalled);
    }

    #[test]
    fn codeword_no_partial_word_match() {
        let s = score("it was sunlight outside", "x", "zzzzzzzz", "sun");
        assert!(!s.codeword_recalled, "부분 단어 매칭 금지");
    }

    #[test]
    fn id_prefix_no_substring_false_positive() {
        let s = score("the word undeadbeefed appeared", "x", "deadbeef0000", "c");
        assert!(
            !s.id_prefix_recalled,
            "id 접두가 무관 단어 내부 substring 으로 매칭되면 안 됨"
        );
        let s2 = score("the id was deadbeef here", "x", "deadbeef0000", "c");
        assert!(s2.id_prefix_recalled, "토큰 경계로 등장하면 회상");
    }

    #[test]
    fn final_report_count_and_title() {
        let s = score_probe(
            "Total documents: 42. DOC-1 title was Silent kernel cascade.",
            "x",
            "zzzzzzzz",
            "c",
            true,
            42,
            "Silent kernel cascade",
        );
        assert!(s.final_count_correct, "문서 수 42 회상");
        assert!(s.doc1_title_recalled, "doc1 제목 회상");
    }

    #[test]
    fn final_report_wrong_count() {
        let s = score_probe(
            "Total documents: 40.",
            "x",
            "zzzzzzzz",
            "c",
            true,
            42,
            "Silent kernel cascade",
        );
        assert!(!s.final_count_correct, "40 != 42");
    }

    #[test]
    fn final_count_no_substring_false_positive() {
        let s = score_probe(
            "I received 41 documents; DOC-42 was not received.",
            "x",
            "zzzzzzzz",
            "c",
            true,
            42,
            "Silent kernel cascade",
        );
        assert!(
            !s.final_count_correct,
            "정답 숫자가 문서 참조(DOC-42)로만 등장하면 회상 아님 — 41 이 실 보고"
        );
    }

    #[test]
    fn final_count_requires_report_cue_nearby() {
        let s = score_probe(
            "The value 42 was mentioned in an unrelated calculation about temperature.",
            "x",
            "zzzzzzzz",
            "c",
            true,
            42,
            "",
        );
        assert!(
            !s.final_count_correct,
            "보고 신호어 없는 벌거벗은 숫자는 회상 아님"
        );
        let s2 = score_probe(
            "In total I received 42 documents.",
            "x",
            "zzzzzzzz",
            "c",
            true,
            42,
            "",
        );
        assert!(s2.final_count_correct, "신호어 근접 42 는 회상");
    }

    #[test]
    fn non_final_report_never_scores_count_or_title() {
        let s = score_probe(
            "42 Silent kernel cascade",
            "x",
            "zzzzzzzz",
            "c",
            false,
            42,
            "Silent kernel cascade",
        );
        assert!(!s.final_count_correct, "비-final 은 항상 false");
        assert!(!s.doc1_title_recalled, "비-final 은 항상 false");
    }

    // ── compaction detection ──

    fn sample(idx: u32, tokens: u64, reset: bool) -> UsageSample {
        UsageSample {
            turn_idx: idx,
            context_tokens: tokens,
            harness_reset: reset,
        }
    }

    #[test]
    fn detects_sharp_drop() {
        // 10000 → 5000 = a 50% drop (>30%) → suspicious.
        let samples = vec![sample(0, 10_000, false), sample(1, 5_000, false)];
        assert_eq!(detect_suspected_compaction(&samples), vec![1]);
    }

    #[test]
    fn ignores_small_drop() {
        // 10000 → 8000 = 20% (<30%) → not suspicious.
        let samples = vec![sample(0, 10_000, false), sample(1, 8_000, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    #[test]
    fn ignores_growth() {
        let samples = vec![sample(0, 5_000, false), sample(1, 12_000, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    #[test]
    fn excludes_harness_reset_drop() {
        let samples = vec![sample(0, 10_000, false), sample(1, 2_000, true)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    #[test]
    fn detects_multiple_drops() {
        let samples = vec![
            sample(0, 10_000, false),
            sample(1, 4_000, false), // 60% drop
            sample(2, 4_100, false), // small increase
            sample(3, 1_000, false), // 76% drop
        ];
        assert_eq!(detect_suspected_compaction(&samples), vec![1, 3]);
    }

    #[test]
    fn zero_prev_is_skipped() {
        let samples = vec![sample(0, 0, false), sample(1, 0, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    // ── finding 2: a single consistent series (real_series_from_footprints) ──

    #[test]
    fn source_transition_drop_does_not_flag_when_series_is_single_source() {
        let estimate_grows_to_30k = real_series_from_footprints(&[8_000u64, 18_000, 30_000]);
        let real_first_5k = [5_000u64, 12_000, 20_000, 33_000, 49_000];

        let selected = select_detection_series(Some(&real_first_5k), &estimate_grows_to_30k);
        assert!(
            selected
                .iter()
                .all(|s| real_first_5k.contains(&s.context_tokens)),
            "real 이 있으면 선택 계열은 real 값만 — 추정값(30000 등)이 섞이면 안 된다"
        );
        assert!(
            !selected.iter().any(|s| s.context_tokens == 30_000),
            "추정 30000 은 감지 계열에 절대 새어 들지 않아야 한다(소스 전환 급감 원천 차단)"
        );
        assert_eq!(
            detect_suspected_compaction(&selected),
            Vec::<u32>::new(),
            "단일 소스(실측) 계열은 소스 전환 인공 급감(30000→5000)이 존재하지 않는다"
        );
    }

    #[test]
    fn select_detection_series_falls_back_to_estimate_when_no_real() {
        let estimate = real_series_from_footprints(&[8_000u64, 18_000, 30_000]);
        assert_eq!(select_detection_series(None, &estimate), estimate);
        assert_eq!(select_detection_series(Some(&[]), &estimate), estimate);
    }

    #[test]
    fn select_detection_series_never_mixes_sources() {
        let estimate = real_series_from_footprints(&[9_999u64, 29_999]);
        let real = [5_000u64, 12_000];
        let selected = select_detection_series(Some(&real), &estimate);
        assert_eq!(selected, real_series_from_footprints(&real));
        assert!(
            !selected
                .iter()
                .any(|s| s.context_tokens == 9_999 || s.context_tokens == 29_999),
            "estimate 값이 real 선택 계열에 섞이면 안 된다"
        );
    }

    #[test]
    fn genuine_real_series_drop_still_flags() {
        // 48000 → 15000 = a 69% drop → idx 2.
        let real_footprints = [10_000u64, 48_000, 15_000, 20_000];
        let samples = real_series_from_footprints(&real_footprints);
        assert_eq!(
            detect_suspected_compaction(&samples),
            vec![2],
            "실 계열 내부의 진짜 급감은 잡아야 한다"
        );
    }

    #[test]
    fn real_series_builder_indexes_and_flags_are_zero_based() {
        let samples = real_series_from_footprints(&[100, 40]); // 60% drop at idx 1.
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].turn_idx, 0);
        assert_eq!(samples[1].turn_idx, 1);
        assert!(!samples[0].harness_reset && !samples[1].harness_reset);
        assert_eq!(detect_suspected_compaction(&samples), vec![1]);
    }
}
