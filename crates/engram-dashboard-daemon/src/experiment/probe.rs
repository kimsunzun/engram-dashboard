//! 프로브 채점 + suspected-compaction 감지 (ADR-0090 Stage 2 파일럿) — 순수 함수.
//!
//! ## 역할
//! (1) **프로브 채점**: 지연 프로브에 대한 에이전트 응답 텍스트를 받아, "지속 처리(sustained
//!     processing)" 판정용 불리언들을 낸다(발신자 회상·msg_id 접두 회상·codeword 회상·최종 문서 수·
//!     doc1 제목 회상). ADR-0088 판정 = 즉시 ack 가 아니라 **지연 후에도 회상 유지**.
//! (2) **suspected-compaction 감지**: 턴별 컨텍스트 토큰 수열을 받아, 하네스가 리셋하지 않았는데
//!     연속 두 턴 사이 토큰이 >30% 급감하면 compaction 의심 플래그를 세운다(claude 의 stream-json 이
//!     compaction 을 명시 신호로 안 줄 때의 외부 근사 — ADR-0090 맥락 "compaction 외부 감지 부재").
//!
//! ## 핵심 불변식
//! - **순수·결정적**: 전부 입력만의 함수(외부 상태·시계 0) — 단위 테스트로 전수 커버.
//! - **채점은 관대한 매칭(대소문자·주변 구두점 무시)** 이되 codeword/제목은 **정답 토큰이 응답에
//!   포함되는가**로 본다. false-positive(우연 포함)보다 false-negative(형식 차이로 놓침) 를 줄이는
//!   방향 — 회상 여부의 신호는 "정답 토큰이 응답 어딘가에 있나" 로 충분하다.
//! - **급감 판정은 harness-reset 을 제외**: 우리가 의도적으로 컨텍스트를 리셋한 지점(예: /compact 발화)
//!   의 급감은 compaction 의심이 아니라 **의도된** 것이므로 `reset` 마킹으로 배제한다.
//!
//! ## 진입점
//! - `score_probe(...)`: 응답 텍스트 → `ProbeScores`.
//! - `detect_suspected_compaction(&[UsageSample])`: 토큰 수열 → 의심 구간 인덱스 목록.
// ADR-0090

use serde::{Deserialize, Serialize};

/// 프로브 응답 1건의 채점 결과(전부 불리언 = 회상 여부). JSONL 에 그대로 실린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeScores {
    /// 발신자 이름을 회상했나.
    pub sender_recalled: bool,
    /// msg_id 앞 8자 이상을 회상했나(전체 uuid 는 길어 접두 회상으로 완화).
    pub id_prefix_recalled: bool,
    /// codeword(정확 토큰)를 회상했나.
    pub codeword_recalled: bool,
    /// 최종 문서 수를 맞췄나(FINAL REPORT 프로브 전용 — 아니면 false).
    pub final_count_correct: bool,
    /// DOC-1 의 정확한 제목을 회상했나(FINAL REPORT 프로브 전용).
    pub doc1_title_recalled: bool,
}

/// msg_id 접두 회상 기준 길이(선행 문자 수). ADR-0090 명세: ≥8 선행 문자.
pub const ID_PREFIX_LEN: usize = 8;

/// 텍스트를 채점용으로 정규화 — 소문자 + 영숫자/공백 외 문자를 공백으로. 구두점·대소문자 차이로 인한
/// false-negative 를 줄인다(회상 신호는 토큰 포함 여부라 이 정규화가 적절).
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

/// `needle` 가 `haystack` 에 **토큰 경계로** 포함되나(정규화 후 공백 구분 부분 시퀀스). 짧은 토큰이
/// 다른 단어의 부분으로 우연 매칭되는 것을 막는다(예: "id" 가 "identity" 에 걸리지 않게).
fn contains_token_seq(haystack_norm: &str, needle: &str) -> bool {
    let needle_norm = normalize(needle);
    let needle_tokens: Vec<&str> = needle_norm.split_whitespace().collect();
    if needle_tokens.is_empty() {
        return false;
    }
    let hay_tokens: Vec<&str> = haystack_norm.split_whitespace().collect();
    // needle 토큰 시퀀스가 hay 토큰 시퀀스의 연속 부분열인가.
    hay_tokens
        .windows(needle_tokens.len())
        .any(|w| w == needle_tokens.as_slice())
}

/// FINAL REPORT 문서 수 채점(finding 11 fix). 정답 숫자가 **count 보고 패턴**으로 등장할 때만 true.
///
/// ★왜 패턴 매칭인가★: 단순 "정답 숫자 토큰이 응답에 있나" 는 "I received 41 documents; DOC-42 was not
///   received"(정답 42) 같은 문장에서 42 가 문서 참조(DOC-42)로 등장해 false-positive 를 낸다. 그래서:
///   (1) 숫자 **바로 앞** 토큰이 `doc`/`document` 면 그건 문서 **참조**(DOC-N)라 count 아님 → 배제.
///   (2) 숫자 **바로 뒤** 토큰이 count 명사(documents/docs)이거나, **바로 앞** 토큰이 보고어
///       (total/received/count/…)면 count 보고로 인정. 인접(±1) 요구로 우연 동반을 차단한다.
fn count_reported_with_cue(haystack_norm: &str, expected_count: u32) -> bool {
    // 숫자 바로 앞에 오면 "count 보고" 신호인 보고어(예: "total 42", "received 42", "documents 42"의
    //   "documents:"→"documents" 처럼 count 명사가 콜론과 함께 숫자 앞에 오는 형태 포함).
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
    // 숫자 바로 뒤에 오면 count 명사인 것(예: "42 documents", "42 docs"). "document"(단수)는 DOC-N
    //   참조에서도 나올 수 있으나, 뒤따르는 위치(숫자 뒤)면 count 명사 용법이라 포함.
    const TRAILING_NOUNS: &[&str] = &["documents", "docs", "document"];
    // 숫자 **바로 앞** 이 이것이면 문서 참조(DOC-N)라 count 아님 — 배제.
    const REFERENCE_LABELS: &[&str] = &["doc", "document"];

    let expected = expected_count.to_string();
    let tokens: Vec<&str> = haystack_norm.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if *tok != expected {
            continue;
        }
        let prev = i.checked_sub(1).map(|j| tokens[j]);
        let next = tokens.get(i + 1).copied();
        // 문서 참조(DOC-N) 배제 — 숫자 바로 앞이 doc/document 라벨.
        if prev.map(|p| REFERENCE_LABELS.contains(&p)).unwrap_or(false) {
            continue;
        }
        // 앞이 보고어 or 뒤가 count 명사면 count 보고로 인정.
        let leading_ok = prev.map(|p| LEADING_CUES.contains(&p)).unwrap_or(false);
        let trailing_ok = next.map(|n| TRAILING_NOUNS.contains(&n)).unwrap_or(false);
        if leading_ok || trailing_ok {
            return true;
        }
    }
    false
}

/// 하나의 지연 프로브 응답을 채점한다.
///
/// - `response`: 에이전트가 프로브에 답한 텍스트(4KB 로 이미 캡된 것을 넘겨도 됨).
/// - `sender_name`: 주입 메시지의 발신자 표시 이름(정답).
/// - `msg_id`: 주입 메시지의 논리 id(정답). 앞 `ID_PREFIX_LEN` 자 이상 포함이면 회상.
/// - `codeword`: 주입 메시지의 codeword(정답 토큰).
/// - `final_report`: 이 프로브가 FINAL REPORT 인가(문서 수·제목 채점 활성).
/// - `expected_doc_count`/`expected_doc1_title`: FINAL REPORT 정답(아니면 무시).
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

    // msg_id 접두 회상. ★finding 12 fix★: raw substring(response.contains(prefix))은 8-hex 접두가 무관
    //   단어 **안쪽**(`deadbeef` in `undeadbeefed`)에 걸려 false-positive 를 낸다. 그래서 **토큰이 접두로
    //   시작하는가**(starts_with)로 본다 — 토큰 경계에서 접두가 시작해야 한다. 이러면 `undeadbeefed`
    //   (접두로 시작 안 함)는 걸러지고, 정답보다 긴 id 토큰(`1a2b3c4d5e`, 접두 `1a2b3c4d` 로 시작)은
    //   정당한 회상으로 잡힌다(에이전트가 전체 id 를 써도 접두 회상 성립).
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
        // ★finding 11 fix★: 정답 숫자가 응답에 **토큰으로만** 등장하면 우연 일치가 잦다("received 41
        //   documents; DOC-42 was not received" 에서 42 가 토큰으로 존재 → 오답을 정답 처리). 그래서
        //   숫자 단독이 아니라 **보고 신호어(total/received/documents/count 등)와 근접 동반**을 요구한다.
        let count_ok = count_reported_with_cue(&norm, expected_doc_count);
        // 제목: 정답 제목 토큰 시퀀스가 응답에 등장하나(doc_title 이 만든 3 토큰).
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

/// 한 턴의 컨텍스트 토큰 샘플. `context_tokens` = claude usage 가 보고한 그 턴의 컨텍스트/입력 토큰 수치
/// (ground truth). `harness_reset` = 이 턴 직전에 하네스가 의도적으로 컨텍스트를 리셋했는가(/compact 등).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageSample {
    /// 턴 인덱스(0-base) — 감지 결과가 어느 경계인지 가리키는 참조.
    pub turn_idx: u32,
    /// 이 턴의 컨텍스트 토큰 수치(claude usage ground truth).
    pub context_tokens: u64,
    /// 이 턴 직전에 하네스가 의도적으로 컨텍스트를 리셋했나(그렇다면 급감은 의심 아님).
    pub harness_reset: bool,
}

/// 급감 임계(비율). 연속 두 턴 사이 컨텍스트 토큰이 이 비율 이상 줄면 의심(ADR-0090: >30%).
pub const COMPACTION_DROP_THRESHOLD: f64 = 0.30;

/// ★finding 2 fix — 단일 일관 계열 빌더★: 실 컨텍스트 footprint 값들(트랜스크립트 탭의 real_usage_series)
///   에서 감지용 UsageSample 계열을 만든다. 인덱스는 계열 내 순번(0-base)이고 harness_reset 은 전부 false
///   (실 리셋 개념이 없는 순수 real 계열).
///
/// ★왜 별도 빌더인가(load-bearing)★: 이전엔 감지 계열(`state.usage_samples`)이 턴마다 "실측 있으면 실측,
///   없으면 추정" 을 섞어 담았다 — 트랜스크립트 탭이 **처음 붙는 순간** 그 턴의 값이 이전 턴의 (더 큰) 추정
///   에서 (더 작은) 실 첫 footprint 로 **소스가 바뀌며** 뚝 떨어져 보였고, 그 인위적 급감이 >30% 면 가짜
///   compaction 플래그가 섰다. 이제 감지는 **한 소스로만 된 계열**(전부 real, 또는 전부 estimate)에서
///   돌린다 — 소스 전환 지점의 인공 급감이 원천적으로 불가능하다. 진짜 real 계열 내부의 급감은 그대로 잡힌다.
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

/// ★finding 2 — 단일 소스 선택 계약(순수 함수)★: 감지를 돌릴 **한 소스** 계열을 고른다. 실측 footprint 가
///   하나라도 있으면 순수 실측 계열(`real_series_from_footprints`)을, 없으면 순수 추정 계열(`estimate`)을
///   그대로 쓴다. **두 계열을 절대 이어 붙이지 않는다** — 그래야 소스 전환(추정→실측) 지점의 인공 급감이
///   원천적으로 생기지 않는다(위 빌더 주석의 버그 재현 참조).
///
/// ★왜 bin 에서 빼서 여기 두나(load-bearing)★: 이 선택 로직이 실측/추정을 섞지 않는다는 보장은 파일럿
///   신뢰성의 핵심인데, 예전엔 bin 안 `match` 에만 있어 단위 테스트가 못 닿았다("never mix" 가 prose 로만
///   존재). 순수 함수로 내려 직접 테스트한다 — real 이 있으면 real-only, 없으면 estimate-only 를 반환하고
///   그 반환 계열에는 소스 전환 급감이 없음을 단언.
pub fn select_detection_series(
    real_footprints: Option<&[u64]>,
    estimate: &[UsageSample],
) -> Vec<UsageSample> {
    match real_footprints {
        Some(fp) if !fp.is_empty() => real_series_from_footprints(fp),
        // 실측 계열 부재(또는 빈 계열) → 순수 추정 계열로 폴백. 추정은 단조 증가라 보통 급감이 없다.
        _ => estimate.to_vec(),
    }
}

/// suspected-compaction 감지 — 토큰 수열에서 하네스-리셋이 아닌 급감(>30%)이 일어난 턴 인덱스들을
/// 돌려준다. 반환 원소 = **급감이 관측된 (뒤쪽) 턴의 turn_idx**(prev→cur 에서 cur).
///
/// ★왜 외부 근사인가★: claude stream-json 이 compaction 을 명시 신호로 안 줄 수 있어(ADR-0090 맥락),
///   컨텍스트 토큰이 리셋 없이 급감하면 내부 compaction 이 일어났다고 **의심**한다. 확증이 아니라
///   플래그다 — 파일럿이 실제 신호(있으면)와 대조할 재료.
pub fn detect_suspected_compaction(samples: &[UsageSample]) -> Vec<u32> {
    let mut flags = Vec::new();
    for pair in samples.windows(2) {
        let prev = &pair[0];
        let cur = &pair[1];
        // 하네스가 의도적으로 리셋한 턴의 급감은 의심 아님(의도된 것) — 배제.
        if cur.harness_reset {
            continue;
        }
        // prev 가 0 이면 비율 계산 불가(아직 컨텍스트 없음) — 급감 판정 스킵.
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
        // 정답 id 가 8자 미만이면 접두 회상 불가(false).
        let s = score("id: abc", "x", "abc", "c");
        assert!(!s.id_prefix_recalled);
    }

    #[test]
    fn id_prefix_matches_leading_eight() {
        // 응답에 앞 8자만 있어도 회상.
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
        // codeword "sun" 이 "sunlight" 의 부분으로 우연 매칭되면 안 됨(토큰 경계).
        let s = score("it was sunlight outside", "x", "zzzzzzzz", "sun");
        assert!(!s.codeword_recalled, "부분 단어 매칭 금지");
    }

    #[test]
    fn id_prefix_no_substring_false_positive() {
        // ★finding 12 regression★: 8-hex 접두 "deadbeef" 가 무관 단어 "undeadbeefed" 안에 substring
        //   으로 걸리면 안 된다(토큰 경계 매칭이라야 함). 정답 접두를 통째로 담은 큰 단어는 회상 아님.
        let s = score("the word undeadbeefed appeared", "x", "deadbeef0000", "c");
        assert!(
            !s.id_prefix_recalled,
            "id 접두가 무관 단어 내부 substring 으로 매칭되면 안 됨"
        );
        // 반면 토큰 경계로 등장하면 회상.
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
        // ★finding 11 regression★: 정답 42 가 "DOC-42 was not received" 처럼 문서 참조로만 등장하고
        //   실제 문서 수는 41 로 보고하면 final_count_correct 는 false 여야 한다(신호어 근접 요구).
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
        // 숫자만 덩그러니 있고 보고 신호어가 멀면 회상 아님.
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
        // 신호어와 근접하면 회상.
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
        // 증가는 절대 의심 아님.
        let samples = vec![sample(0, 5_000, false), sample(1, 12_000, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    #[test]
    fn excludes_harness_reset_drop() {
        // 리셋 턴의 급감은 의도된 것 → 배제.
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
        // prev 0 → 비율 계산 불가라 스킵(패닉/분모0 방지).
        let samples = vec![sample(0, 0, false), sample(1, 0, false)];
        assert_eq!(detect_suspected_compaction(&samples), Vec::<u32>::new());
    }

    // ── finding 2: 단일 일관 계열(real_series_from_footprints) ──

    #[test]
    fn source_transition_drop_does_not_flag_when_series_is_single_source() {
        // ★finding 2 regression★: 실제 버그 시나리오를 두 후보 계열로 재현한다 — 추정 계열은 커져서 30000 까지
        //   갔고(estimate), 트랜스크립트 탭이 처음 붙는 순간의 실 첫 footprint 는 그보다 훨씬 작은 5000 이다
        //   (real). 예전 혼합 계열은 이 둘을 이어 붙여 30000→5000(83% 급감)을 가짜 compaction 으로 플래그했다.
        //
        //   핵심 단언: 선택기(select_detection_series)는 real 이 있으면 real-only 를 고르므로, 감지가 도는
        //   계열에는 30000(추정)이 애초에 없다 → 30000→5000 급감은 관측 불가. estimate 계열이 30000 까지
        //   컸다는 사실은 감지에 절대 새어 들지 않아야 한다("never mix").
        let estimate_grows_to_30k = real_series_from_footprints(&[8_000u64, 18_000, 30_000]);
        let real_first_5k = [5_000u64, 12_000, 20_000, 33_000, 49_000];

        let selected = select_detection_series(Some(&real_first_5k), &estimate_grows_to_30k);
        // 선택된 계열은 real-only 여야 한다(추정 30000 이 섞이지 않음).
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
        // 실측 footprint 가 없으면(None 또는 빈 계열) 순수 추정 계열을 그대로 쓴다.
        let estimate = real_series_from_footprints(&[8_000u64, 18_000, 30_000]);
        assert_eq!(select_detection_series(None, &estimate), estimate);
        assert_eq!(select_detection_series(Some(&[]), &estimate), estimate);
    }

    #[test]
    fn select_detection_series_never_mixes_sources() {
        // ★"never mix" 계약을 직접 단언★: real 이 하나라도 있으면 반환 계열은 오직 real 값들로만 구성되고,
        //   estimate 의 어떤 값도 포함하지 않는다(설령 estimate 가 더 크더라도).
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
        // ★finding 2★: 진짜 실측 계열 안의 급감(예: native compaction 으로 컨텍스트가 실제 줄어듦)은 여전히
        //   플래그돼야 한다. 48000 → 15000 (69% 급감) → idx 2 플래그.
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
        // 빌더가 계열 순번을 0-base turn_idx 로 넣고, harness_reset 은 전부 false(순수 실측).
        let samples = real_series_from_footprints(&[100, 40]); // 60% drop at idx 1.
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].turn_idx, 0);
        assert_eq!(samples[1].turn_idx, 1);
        assert!(!samples[0].harness_reset && !samples[1].harness_reset);
        assert_eq!(detect_suspected_compaction(&samples), vec![1]);
    }
}
