//! 프로브 채점 + suspected-compaction 감지 (ADR-0090 Stage 2 파일럿).
//!
//! ## 핵심 불변식
//! - **순수·결정적** — 전부 입력만의 함수다(외부 상태·시계 0).
//! - **판정 = 지연 후에도 회상 유지**(ADR-0088) — 즉시 ack 는 성공이 아니다. `ProbeScores` 가 그 인코딩.

use serde::{Deserialize, Serialize};

/// 프로브 응답 1건의 채점 결과. JSONL 에 그대로 실린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeScores {
    pub sender_recalled: bool,
    /// 전체 uuid 는 길어 접두 회상으로 완화한 축.
    pub id_prefix_recalled: bool,
    pub codeword_recalled: bool,
    /// FINAL REPORT 프로브 전용 — 아니면 항상 false.
    pub final_count_correct: bool,
    /// DOC-1 제목. FINAL REPORT 프로브 전용 — 아니면 항상 false.
    pub doc1_title_recalled: bool,
}

/// msg_id 접두 회상 기준 길이(선행 바이트).
pub const ID_PREFIX_LEN: usize = 8;

/// 구두점·대소문자 차이로 인한 false-negative 를 줄인다.
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

/// 짧은 토큰이 다른 단어의 부분으로 우연 매칭되는 것을 막는다(예: "id" 가 "identity" 에 걸리지 않게).
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

/// ★왜 패턴 매칭인가(finding 11 fix)★: 단순 "정답 숫자 토큰이 응답에 있나" 는 "I received 41 documents;
///   DOC-42 was not received"(정답 42) 같은 문장에서 42 가 문서 **참조**(DOC-42)로 등장해 false-positive
///   를 낸다. 그래서 정답 숫자가 count 보고로 읽히는 인접(±1) 문맥을 요구한다.
fn count_reported_with_cue(haystack_norm: &str, expected_count: u32) -> bool {
    // count 명사가 여기 있는 이유: "Total documents: 42" 가 정규화로 "total documents 42" 가 돼 count
    //   명사가 숫자 **앞**에 선다.
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
    // "document"(단수)는 DOC-N 참조에도 나오지만, 숫자 **뒤**에 서면 count 명사 용법이라 포함한다.
    const TRAILING_NOUNS: &[&str] = &["documents", "docs", "document"];
    // 숫자 **앞**에 서면 문서 참조(DOC-N)라 count 가 아니다 — 배제.
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

/// `response` 만 에이전트 산출이고 `sender_name`·`msg_id`·`codeword`·`expected_*` 는 전부 **정답**이다.
/// `expected_doc_count`/`expected_doc1_title` 은 `final_report` 가 false 면 무시된다.
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

    // ★finding 12 fix★: raw substring(`response.contains(prefix)`)은 8-hex 접두가 무관 단어 **안쪽**
    //   (`deadbeef` in `undeadbeefed`)에 걸려 false-positive 를 냈다 — 완화 금지. 동시에 equality 가 아니라
    //   starts_with 라, 에이전트가 전체 id(`1a2b3c4d5e`)를 써도 접두 회상이 성립한다.
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
    /// claude usage 가 보고한 그 턴의 컨텍스트 토큰(ground truth).
    pub context_tokens: u64,
    /// 이 턴 직전에 하네스가 의도적으로 컨텍스트를 리셋했나 — 그렇다면 급감은 의심이 아니다.
    pub harness_reset: bool,
}

/// 연속 두 턴 사이 컨텍스트 토큰이 이 비율을 **넘게** 줄면 의심.
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

/// ★finding 2 — 단일 소스 선택 계약★: 감지를 돌릴 **한 소스** 계열을 고른다. **두 계열을 절대 이어
///   붙이지 않는다.**
///
/// ★왜(load-bearing)★: 혼합 계열은 트랜스크립트 탭이 **처음 붙는 순간** 그 턴 값이 이전 턴의 (더 큰)
///   추정에서 (더 작은) 실 첫 footprint 로 소스가 바뀌며 뚝 떨어졌고, 그 인위적 급감이 임계를 넘어 가짜
///   compaction 플래그가 섰다. 단일 소스면 그 전환 자체가 없다 — 진짜 급감은 그대로 잡힌다. 이 보장이
///   bin 안 `match` 에만 있던 시절엔 prose 로만 존재해 단위 테스트가 못 닿았다.
pub fn select_detection_series(
    real_footprints: Option<&[u64]>,
    estimate: &[UsageSample],
) -> Vec<UsageSample> {
    match real_footprints {
        Some(fp) if !fp.is_empty() => real_series_from_footprints(fp),
        _ => estimate.to_vec(),
    }
}

/// 반환 원소 = **급감이 관측된 (뒤쪽) 턴의 turn_idx**(prev→cur 에서 cur).
///
/// ★왜 외부 근사인가★: claude stream-json 이 compaction 을 명시 신호로 안 줄 수 있어(ADR-0090 맥락),
///   컨텍스트 토큰이 리셋 없이 급감하면 내부 compaction 을 **의심**한다. 확증이 아니라 플래그다 —
///   파일럿이 실제 신호(있으면)와 대조할 재료.
pub fn detect_suspected_compaction(samples: &[UsageSample]) -> Vec<u32> {
    let mut flags = Vec::new();
    for pair in samples.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        if cur.harness_reset {
            continue;
        }
        // prev 가 0 이면 비율이 무의미하다 — 스킵.
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

    // ── compaction 감지 ──

    fn sample(idx: u32, tokens: u64, reset: bool) -> UsageSample {
        UsageSample {
            turn_idx: idx,
            context_tokens: tokens,
            harness_reset: reset,
        }
    }

    #[test]
    fn detects_sharp_drop() {
        // 10000 → 5000 = 50% 급감(>30%) → 의심.
        let samples = vec![sample(0, 10_000, false), sample(1, 5_000, false)];
        assert_eq!(detect_suspected_compaction(&samples), vec![1]);
    }

    #[test]
    fn ignores_small_drop() {
        // 10000 → 8000 = 20% (<30%) → 의심 아님.
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
            sample(1, 4_000, false), // 급감 60%
            sample(2, 4_100, false), // 소폭 증가
            sample(3, 1_000, false), // 급감 76%
        ];
        assert_eq!(detect_suspected_compaction(&samples), vec![1, 3]);
    }

    #[test]
    fn zero_prev_is_skipped() {
        let samples = vec![sample(0, 0, false), sample(1, 0, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    // ── finding 2: 단일 일관 계열(real_series_from_footprints) ──

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
        // 48000 → 15000 = 69% 급감 → idx 2.
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
