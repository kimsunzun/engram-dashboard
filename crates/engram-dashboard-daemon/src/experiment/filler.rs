//! 결정적 필러 문서 생성기 (ADR-0090 Stage 2 파일럿).
//!
//! ## 역할
//! 컨텍스트 포화용 **자연어 산문(natural prose)** 문서를 **결정적(seed 고정)** 으로 만든다. 같은 seed·
//! 같은 doc 번호·같은 목표 길이면 런마다 **바이트 단위로 동일한** 문서가 나온다(재현성 핀 = ADR-0088 d5a).
//!
//! ## ★content-filter 안전(파일럿 발견 2026-07-20)★
//! 초기 xorshift pseudo-prose(단어 사전에서 난수로 뽑아 나열)는 claude 의 content filter 에 "violates
//! Usage Policy" 로 걸려 스모크 런이 거부됐다. 무의미한 토큰 난열이 의심 신호로 잡힌 것으로 추정. 대체:
//! **템플릿 기반 자연어**(물류 보고서·기상 일지·시설 점검 노트 — 문법적 영어 문장).
//!
//! ## 핵심 불변식
//! - **결정성**: 난수원은 seed 로 시드된 xorshift64 하나뿐 — 외부 상태·시계·env 참조 0.
//! - **문법적 자연어**: 본문의 모든 문장은 `SENTENCE_TEMPLATES` 중 하나를 슬롯 채운 것이다 — content
//!   filter 안전 + "no gibberish" 단위 테스트가 이 불변식을 검증한다.
//! - **근사 길이**: `approx_chars` 는 목표치일 뿐 정확치가 아니다 — 문단 경계에서 끊으므로 ±한 문단
//!   오차가 난다. 포화 루프는 우리가 보낸 누적 문자수(우리 통제)를 진행 신호로 쓰므로 이 근사면 충분하다.

/// 인라인 xorshift64 PRNG(SplitMix 계열 상수로 시드) — 통계적 품질은 요구하지 않는다(슬롯 선택
/// 다양성만 필요).
pub struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    /// seed 로 시드. seed==0 이면 xorshift64 가 고정점(항상 0)이라 SplitMix 상수로 대체해 비-0 보장.
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// n==0 분기는 방어적 — 호출자는 비-0 만 넘긴다.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn pick<'a>(&mut self, slice: &[&'a str]) -> &'a str {
        if slice.is_empty() {
            ""
        } else {
            slice[self.below(slice.len())]
        }
    }
}

// ── benign 어휘 슬롯 ──────────────────────────────────────────────────────────────────

const PLACES: &[&str] = &[
    "warehouse",
    "loading dock",
    "north corridor",
    "storage bay",
    "receiving area",
    "cold room",
    "maintenance shed",
    "control room",
    "packing line",
    "outer yard",
    "inspection station",
    "utility annex",
    "shipping office",
    "east wing",
    "pump house",
    "sorting hall",
];

const ITEMS: &[&str] = &[
    "pallet",
    "container",
    "coolant valve",
    "ventilation duct",
    "conveyor belt",
    "shelving unit",
    "circuit panel",
    "water pump",
    "cargo crate",
    "safety rail",
    "loading ramp",
    "temperature sensor",
    "fire hydrant",
    "backup generator",
    "drainage channel",
    "cabinet",
];

const CONDITIONS: &[&str] = &[
    "operational",
    "stable",
    "within tolerance",
    "clean and dry",
    "properly labeled",
    "fully stocked",
    "recently serviced",
    "securely fastened",
    "clearly marked",
    "in good order",
    "well ventilated",
    "correctly aligned",
];

const ACTIONS: &[&str] = &[
    "inspected",
    "recorded",
    "measured",
    "verified",
    "cleaned",
    "restocked",
    "calibrated",
    "logged",
    "surveyed",
    "reviewed",
    "counted",
    "documented",
];

const WEATHER: &[&str] = &[
    "clear and mild",
    "overcast with light wind",
    "cool and dry",
    "humid but stable",
    "calm throughout the day",
    "breezy in the afternoon",
    "steady with no precipitation",
    "warm with scattered clouds",
];

const ROLES: &[&str] = &[
    "the day shift team",
    "the maintenance crew",
    "the inspection officer",
    "the logistics coordinator",
    "the site supervisor",
    "the receiving clerk",
    "the safety inspector",
    "the warehouse operator",
];

const SENTENCE_TEMPLATES: &[&str] = &[
    "The {item} in the {place} was {action} and found to be {cond}.",
    "During the morning round, {role} checked that the {item} remained {cond}.",
    "Weather at the site was {weather}, so outdoor work near the {place} proceeded on schedule.",
    "A routine count confirmed that every {item} in the {place} was {cond}.",
    "{role} {action} the {item} and noted no irregularities.",
    "Conditions in the {place} stayed {cond} throughout the shift.",
    "The report indicates that the {item} was {action} before the {place} was closed for the day.",
    "Because the weather turned {weather}, {role} moved the {item} into the {place}.",
    "Each {item} was {action}, labeled, and stored in the {place} without incident.",
    "The inspection of the {place} showed the {item} to be {cond} and ready for use.",
];

/// 본문 어휘와 분리해 제목이 시각적으로 구분되게 한다.
const TITLE_ADJ: &[&str] = &[
    "Northern",
    "Coastal",
    "Central",
    "Quarterly",
    "Regional",
    "Riverside",
    "Highland",
    "Eastern",
    "Summit",
    "Harbor",
    "Meadow",
    "Valley",
    "Lakeside",
    "Western",
    "Autumn",
    "Morning",
];

const TITLE_NOUN: &[&str] = &[
    "logistics",
    "inspection",
    "facility",
    "weather",
    "storage",
    "maintenance",
    "shipping",
    "warehouse",
    "operations",
    "safety",
    "inventory",
    "survey",
    "depot",
    "records",
];

/// doc n 의 **결정적 제목**. `(seed, n)` 만의 함수 — 드라이버가 이 함수를 그대로 불러 프로브 정답
/// (doc1_title_recalled)을 재구성하므로, 제목 산출 규칙은 여기 하나에만 존재한다(단일 출처).
///
/// 세 토큰이라 claude 가 정확 회상하기엔 적당히 어렵고(그냥 "the document" 로 뭉개면 실패), 채점은
/// exact match 로 명확하다.
pub fn doc_title(seed: u64, n: u32) -> String {
    let mut rng = Xorshift64::new(seed ^ (0xD1B5_4A32_D192_ED03u64.wrapping_mul(n as u64 + 1)));
    let adj = rng.pick(TITLE_ADJ);
    let noun1 = rng.pick(TITLE_NOUN);
    let noun2 = rng.pick(TITLE_NOUN);
    format!("{adj} {noun1} {noun2}")
}

/// doc n 의 완결 본문 — `DOC-<n>: <제목>\n` 헤더 + 자연어 문단들.
///
/// 본문 PRNG 는 제목과 다른 교란 시드라 제목/본문이 독립적이다(제목만 회상하고 본문은 못 하는 상황을
/// 분리 관측 가능).
pub fn filler_doc(seed: u64, n: u32, approx_chars: usize) -> String {
    // ★방어적 상한(finding 7)★: approx_chars 가 폭주 값(usize::MAX 등)이면 with_capacity 예약·문단 루프가
    //   즉시 OOM/panic 이다. CLI 가 이미 클램프하지만, 이 순수 함수를 직접 부르는 경로(테스트·미래
    //   호출자)도 있으니 여기서도 하드 상한을 건다.
    let approx_chars = approx_chars.min(super::cli::DOC_CHARS_CLAMP);
    let title = doc_title(seed, n);
    let mut out = String::with_capacity(approx_chars + 256);
    out.push_str(&format!("DOC-{n}: {title}\n"));

    let mut rng = Xorshift64::new(seed ^ (0x2545_F491_4F6C_DD1Du64.wrapping_mul(n as u64 + 1)));
    while out.len() < approx_chars {
        out.push_str(&paragraph(&mut rng));
        out.push_str("\n\n");
    }
    out
}

fn paragraph(rng: &mut Xorshift64) -> String {
    let sentences = 3 + rng.below(4);
    let mut p = String::new();
    for i in 0..sentences {
        if i > 0 {
            p.push(' ');
        }
        p.push_str(&fill_template(rng));
    }
    p
}

/// `{role}` 이 문두인 템플릿은 role 어휘의 첫 글자가 소문자("the ...")이므로 대문자화가 필요하다.
fn fill_template(rng: &mut Xorshift64) -> String {
    // ★슬롯 채우기 순서 고정★: **슬롯 타입별 고정 순서**로 뽑으면 안 된다(문장마다 슬롯 조합이 달라
    //   PRNG 소비량이 흔들려 재현성이 깨진다). 템플릿 문자열을 왼→오로 스캔하며 등장하는 슬롯마다
    //   순차로 뽑는다(등장 순서 = 소비 순서 고정).
    let template = rng.pick(SENTENCE_TEMPLATES);
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = match after.find('}') {
            Some(c) => c,
            None => {
                // 닫는 중괄호 없음 — 있을 수 없는 템플릿에 대한 방어.
                out.push_str(&rest[open..]);
                rest = "";
                break;
            }
        };
        let slot = &after[..close];
        let value = match slot {
            "place" => rng.pick(PLACES),
            "item" => rng.pick(ITEMS),
            "cond" => rng.pick(CONDITIONS),
            "action" => rng.pick(ACTIONS),
            "weather" => rng.pick(WEATHER),
            "role" => rng.pick(ROLES),
            _ => "", // 미지 슬롯 — 있을 수 없음.
        };
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    capitalize_first(&out)
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::with_capacity(s.len());
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_deterministic_for_same_seed() {
        let mut a = Xorshift64::new(42);
        let mut b = Xorshift64::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64(), "같은 seed 는 같은 수열");
        }
    }

    #[test]
    fn xorshift_seed_zero_is_nonzero_stream() {
        let mut r = Xorshift64::new(0);
        assert_ne!(
            r.next_u64(),
            0,
            "seed 0 이 0 수열이 되면 안 됨(고정점 회피)"
        );
    }

    #[test]
    fn doc_title_is_deterministic() {
        let t1 = doc_title(7, 3);
        let t2 = doc_title(7, 3);
        assert_eq!(t1, t2, "같은 (seed, n) → 같은 제목");
        assert_eq!(t1.split(' ').count(), 3, "제목은 3 토큰: {t1:?}");
    }

    #[test]
    fn doc_titles_differ_across_n() {
        let titles: Vec<String> = (1..=10).map(|n| doc_title(99, n)).collect();
        let distinct: std::collections::HashSet<_> = titles.iter().collect();
        assert!(
            distinct.len() >= 5,
            "10개 doc 제목 중 최소 5개는 distinct 여야(다양성): {titles:?}"
        );
    }

    #[test]
    fn filler_doc_header_format_and_title_match() {
        let doc = filler_doc(123, 1, 500);
        let title = doc_title(123, 1);
        let expected_header = format!("DOC-1: {title}");
        assert!(
            doc.starts_with(&expected_header),
            "헤더가 `DOC-<n>: <제목>` 이어야 하고 제목이 doc_title 과 일치: 첫줄={:?}",
            doc.lines().next()
        );
        assert!(doc[expected_header.len()..].starts_with('\n'));
    }

    #[test]
    fn filler_doc_is_deterministic_byte_for_byte() {
        let a = filler_doc(0xABCD, 5, 3000);
        let b = filler_doc(0xABCD, 5, 3000);
        assert_eq!(
            a, b,
            "같은 (seed, n, approx_chars) → 바이트 동일(재현성 핀)"
        );
    }

    #[test]
    fn filler_doc_approx_length_reached() {
        let approx = 4000;
        let doc = filler_doc(1, 2, approx);
        assert!(doc.len() >= approx, "최소 목표 길이 도달: {}", doc.len());
        assert!(
            doc.len() < approx + 2000,
            "초과가 한 문단 규모여야(폭주 아님): {}",
            doc.len()
        );
    }

    #[test]
    fn filler_docs_differ_across_n() {
        let d1 = filler_doc(5, 1, 2000);
        let d2 = filler_doc(5, 2, 2000);
        assert_ne!(d1, d2, "다른 doc 번호는 다른 본문");
    }

    #[test]
    fn filler_doc_clamps_absurd_length() {
        let doc = filler_doc(1, 1, usize::MAX);
        assert!(
            doc.len() < super::super::cli::DOC_CHARS_CLAMP + 4000,
            "폭주 길이가 클램프+한 문단 규모로 제한: {}",
            doc.len()
        );
        assert!(
            doc.len() >= super::super::cli::DOC_CHARS_CLAMP,
            "클램프까지는 채움"
        );
    }

    #[test]
    fn filler_doc_is_valid_utf8_prose() {
        let doc = filler_doc(3, 4, 1000);
        assert!(doc.is_ascii(), "ASCII 만 사용(멀티바이트 이슈 회피)");
        assert!(doc.contains('.'), "문장 마침표 존재");
    }

    #[test]
    fn every_sentence_matches_a_template() {
        let mut sentences: Vec<String> = Vec::new();
        for seed in [1u64, 42, 0xABCD, 7] {
            for n in 1..=3u32 {
                let doc = filler_doc(seed, n, 2500);
                for para in doc.lines().skip(1) {
                    for raw in para.split(". ") {
                        let s = raw.trim();
                        if s.is_empty() {
                            continue;
                        }
                        let s = s.trim_end_matches('.').to_string();
                        if !s.is_empty() {
                            sentences.push(s);
                        }
                    }
                }
            }
        }
        assert!(sentences.len() > 30, "충분한 표본: {}", sentences.len());
        for s in &sentences {
            assert!(
                sentence_matches_any_template(s),
                "gibberish 문장(어느 템플릿과도 불일치): {s:?}"
            );
        }
    }

    fn sentence_matches_any_template(sentence: &str) -> bool {
        let sent_lower = sentence.to_ascii_lowercase();
        SENTENCE_TEMPLATES.iter().any(|t| {
            let t = t.trim_end_matches('.').to_ascii_lowercase();
            let mut cursor = 0usize;
            let mut rest: &str = &t;
            let mut ok = true;
            while let Some(open) = rest.find('{') {
                let literal = &rest[..open];
                if !literal.is_empty() {
                    match sent_lower[cursor..].find(literal.trim()) {
                        Some(pos) if !literal.trim().is_empty() => {
                            cursor += pos + literal.trim().len()
                        }
                        _ => {
                            if !literal.trim().is_empty() {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
                let after = &rest[open + 1..];
                match after.find('}') {
                    Some(c) => rest = &after[c + 1..],
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && !rest.trim().is_empty() {
                if let Some(pos) = sent_lower[cursor..].find(rest.trim()) {
                    let _ = pos;
                } else {
                    ok = false;
                }
            }
            ok
        })
    }
}
