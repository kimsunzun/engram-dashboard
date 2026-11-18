//! Probe grading + suspected-compaction detection (ADR-0090 Stage 2 pilot).
//!
//! ## Core invariants
//! - Pure and deterministic — everything is a function of the input alone (no external state or clock).
//! - Decision = recall holds even after a delay (ADR-0088) — an immediate ack is not success. `ProbeScores` encodes that.

use serde::{Deserialize, Serialize};

/// Grading result of one probe response, carried as is in the JSONL.
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

/// Prevents a short token from matching as part of another word (e.g. "id" in "identity").
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

/// ★Why pattern matching★: a plain "is the correct number token in the response" gives a false positive on "I received 41 documents; DOC-42 was not received" (answer 42), where 42 is a document reference (DOC-42). So require an adjacent (±1) context where the correct number reads as a count report.
fn count_reported_with_cue(haystack_norm: &str, expected_count: u32) -> bool {
    // "Total documents: 42" normalizes to "total documents 42", so the count noun stands before the number.
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
    // "document" (singular) also appears in DOC-N references, but after the number it is the count-noun usage, so it is included.
    const TRAILING_NOUNS: &[&str] = &["documents", "docs", "document"];
    // Before the number it is a document reference (DOC-N), not a count — excluded.
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

/// Only `response` is agent output; `sender_name`·`msg_id`·`codeword`·`expected_*` are all answers. `expected_doc_count`/`expected_doc1_title` are ignored if `final_report` is false.
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

    // ★Do not relax★: a raw substring (`response.contains(prefix)`) gave a false positive when the 8-hex prefix landed inside an unrelated word (`deadbeef` in `undeadbeefed`). It is starts_with, not equality, so prefix recall holds even if the agent writes the full id.
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

/// Suspicious if the context tokens between two consecutive turns fall by more than this ratio.
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

/// ★Single-source selection contract★: choose one source series to run detection on. ★Never join two series.★
///
/// ★Why (load-bearing)★: in a mixed series, at the moment the transcript tap first attaches, the turn's value dropped sharply as the source switched from the previous turn's (larger) estimate to the (smaller) real footprint, and that artificial drop crossed the threshold and raised a fake compaction flag. With a single source the switch does not exist, and a real drop is still caught. This guarantee existed only as a `match` in the bin, out of unit-test reach.
pub fn select_detection_series(
    real_footprints: Option<&[u64]>,
    estimate: &[UsageSample],
) -> Vec<UsageSample> {
    match real_footprints {
        Some(fp) if !fp.is_empty() => real_series_from_footprints(fp),
        _ => estimate.to_vec(),
    }
}

/// Returned element = the turn_idx of the (later) turn where the drop was observed (cur in prev→cur).
///
/// ★Why an external approximation★: claude stream-json may not give compaction as an explicit signal (ADR-0090), so a sharp context drop without a reset makes us suspect internal compaction. A flag, not confirmation — material for cross-checking against the real signal (if any).
pub fn detect_suspected_compaction(samples: &[UsageSample]) -> Vec<u32> {
    let mut flags = Vec::new();
    for pair in samples.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        if cur.harness_reset {
            continue;
        }
        // prev = 0 makes the ratio meaningless — skip.
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
        assert!(s.sender_recalled, "sender recalled");
        assert!(s.id_prefix_recalled, "id prefix recalled");
        assert!(s.codeword_recalled, "codeword recalled");
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
        assert!(!s.codeword_recalled, "no partial word matching");
    }

    #[test]
    fn id_prefix_no_substring_false_positive() {
        let s = score("the word undeadbeefed appeared", "x", "deadbeef0000", "c");
        assert!(
            !s.id_prefix_recalled,
            "the id prefix must not match as a substring inside an unrelated word"
        );
        let s2 = score("the id was deadbeef here", "x", "deadbeef0000", "c");
        assert!(
            s2.id_prefix_recalled,
            "recalled when it appears at a token boundary"
        );
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
        assert!(s.final_count_correct, "doc count 42 recalled");
        assert!(s.doc1_title_recalled, "doc1 title recalled");
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
            "if the correct number appears only as a doc reference (DOC-42) it is not recalled — 41 is the real report"
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
            "a bare number without a report signal word is not recalled"
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
        assert!(s2.final_count_correct, "42 near a signal word is recalled");
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
        assert!(!s.final_count_correct, "non-final is always false");
        assert!(!s.doc1_title_recalled, "non-final is always false");
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

    // ── a single consistent series (real_series_from_footprints) ──

    #[test]
    fn source_transition_drop_does_not_flag_when_series_is_single_source() {
        let estimate_grows_to_30k = real_series_from_footprints(&[8_000u64, 18_000, 30_000]);
        let real_first_5k = [5_000u64, 12_000, 20_000, 33_000, 49_000];

        let selected = select_detection_series(Some(&real_first_5k), &estimate_grows_to_30k);
        assert!(
            selected
                .iter()
                .all(|s| real_first_5k.contains(&s.context_tokens)),
            "if real exists, the chosen series has real values only — estimates (30000 etc.) must not mix in"
        );
        assert!(
            !selected.iter().any(|s| s.context_tokens == 30_000),
            "the estimate 30000 must never leak into the detection series (blocks source-switch drops at the root)"
        );
        assert_eq!(
            detect_suspected_compaction(&selected),
            Vec::<u32>::new(),
            "a single-source (measured) series has no artificial source-switch drop (30000→5000)"
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
            "estimate values must not mix into the real chosen series"
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
            "a genuine drop inside the real series must be caught"
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
