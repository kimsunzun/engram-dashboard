//! 결정적 필러 문서 생성기 (ADR-0090 Stage 2 파일럿).
//!
//! ## 역할
//! 컨텍스트 포화용 **자연어 산문(natural prose)** 문서를 **결정적(seed 고정)** 으로 만든다. 같은 seed·
//! 같은 doc 번호·같은 목표 길이면 런마다 **바이트 단위로 동일한** 문서가 나온다(재현성 핀 = ADR-0088 d5a).
//!
//! ## ★content-filter 안전(파일럿 발견 2026-07-20)★
//! 초기 xorshift pseudo-prose(단어 사전에서 난수로 뽑아 나열)는 claude 의 content filter 에 "violates
//! Usage Policy" 로 걸려 스모크 런이 거부됐다. 무의미한 토큰 난열이 의심 신호로 잡힌 것으로 추정. 대체:
//! **템플릿 기반 자연어**(물류 보고서·기상 일지·시설 점검 노트 — 문법적 영어 문장). 각 문장은 고정
//! 템플릿의 슬롯을 benign 어휘로 결정적으로 채운 것이라 항상 문법적이고 무해하다. 여전히 seed 만의
//! 함수(결정성 유지). 옛 xorshift PRNG(`Xorshift64`)는 슬롯 선택의 난수원으로 그대로 재사용한다.
//!
//! ## 핵심 불변식
//! - **결정성**: 난수원은 seed 로 시드된 xorshift64 하나뿐(외부 상태·시계·env 참조 0). doc n 의 내용은
//!   `(seed, n, approx_chars)` 만의 함수다.
//! - **문서 헤더 계약**: doc n 은 항상 `DOC-<n>: <제목>\n` 으로 시작한다. 제목은 결정적(단어 사전에서
//!   seed+n 로 뽑음)이라 프로브(doc1_title_recalled)가 정답을 재구성할 수 있다 — 헤더 포맷·제목 산출은
//!   `doc_title` 하나가 단일 출처다(프로브 채점이 같은 함수를 재사용해 정답을 만든다).
//! - **문법적 자연어**: 본문의 모든 문장은 `SENTENCE_TEMPLATES` 중 하나를 슬롯 채운 것이다 — content
//!   filter 안전 + "no gibberish" 단위 테스트가 이 불변식을 검증한다.
//! - **근사 길이**: `approx_chars` 는 목표치일 뿐 정확치가 아니다 — 문단 경계에서 끊으므로 ±한 문단
//!   오차가 난다. 포화 루프는 우리가 보낸 누적 문자수(우리 통제)를 진행 신호로 쓰므로 이 근사면 충분하다.
//!
//! ## 진입점
//! - `Xorshift64`: 인라인 tiny PRNG(슬롯 선택 난수원 — SplitMix 상수 시드).
//! - `doc_title(seed, n)`: doc n 의 결정적 제목(프로브 정답 재구성에도 쓰임).
//! - `filler_doc(seed, n, approx_chars)`: doc n 의 완결 본문(헤더 + 자연어 문단들).
// ADR-0090

/// 인라인 xorshift64 PRNG. stdio_physical_pipe.rs 의 선례와 동일 알고리즘(SplitMix 계열 상수로 시드) —
/// 결정적·의존성 0. 통계적 품질은 요구하지 않는다(슬롯 선택 다양성만 필요). seed 0 은 xorshift 가 영원히
/// 0 을 뱉으므로 생성자가 비-0 으로 보정한다.
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

    /// 다음 u64. xorshift64 표준 시프트(13/7/17).
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// [0, n) 범위 usize. n==0 이면 0(방어적 — 호출자는 비-0 만 넘김).
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// 슬라이스에서 결정적으로 1개 뽑기(빈 슬라이스면 "").
    fn pick<'a>(&mut self, slice: &[&'a str]) -> &'a str {
        if slice.is_empty() {
            ""
        } else {
            slice[self.below(slice.len())]
        }
    }
}

// ── benign 어휘 슬롯(전부 무해한 실무 도메인 명사·형용사) ────────────────────────────────
//   물류/기상/시설 점검 도메인 — content filter 가 문제 삼을 표현이 없다(사물·수치·절차만).

/// 장소·구역 명사(슬롯 {place}).
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

/// 대상 사물 명사(슬롯 {item}).
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

/// 상태·품질 형용사(슬롯 {cond}).
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

/// 동작 동사구(슬롯 {action}).
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

/// 기상·환경 형용사(슬롯 {weather}).
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

/// 사람 역할(슬롯 {role}).
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

/// 문장 템플릿. 각 원소는 슬롯 토큰(`{place}` 등)을 가진 문법적 영어 문장이다. 슬롯은
/// `fill_template` 이 위 benign 어휘에서 결정적으로 채운다. **모든 본문 문장은 이 템플릿 중 하나** —
/// "no gibberish" 단위 테스트가 이를 보장한다(content filter 안전의 근거).
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

/// doc 제목의 첫 단어 후보(형용사). 본문 어휘와 분리해 제목이 시각적으로 구분되게 한다.
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

/// doc 제목의 명사 후보(도메인 명사).
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

/// doc n 의 **결정적 제목**. `(seed, n)` 만의 함수 — 프로브 채점(doc1_title_recalled)이 이 함수를 그대로
/// 불러 정답을 재구성하므로, 제목 산출 규칙은 여기 하나에만 존재한다(단일 출처).
///
/// 형식: `<형용사> <명사> <명사>`(예: "Northern logistics survey"). 세 토큰이라 claude 가 정확 회상하기엔
/// 적당히 어렵고(그냥 "the document" 로 뭉개면 실패), 채점은 exact match 로 명확하다. 전부 benign
/// 실무 어휘라 content filter 안전.
pub fn doc_title(seed: u64, n: u32) -> String {
    // 제목 전용 PRNG 서브스트림 — 본문 생성 PRNG 와 섞이지 않게 seed 를 doc 번호로 교란한 별도 시드.
    //   (본문은 filler_doc 이 또 다른 교란 시드로 돌린다 — 제목/본문 독립.)
    let mut rng = Xorshift64::new(seed ^ (0xD1B5_4A32_D192_ED03u64.wrapping_mul(n as u64 + 1)));
    let adj = rng.pick(TITLE_ADJ);
    let noun1 = rng.pick(TITLE_NOUN);
    let noun2 = rng.pick(TITLE_NOUN);
    format!("{adj} {noun1} {noun2}")
}

/// doc n 의 완결 본문. `DOC-<n>: <제목>\n` 헤더 + 자연어 문단들(목표 `approx_chars` 근사).
///
/// ★결정성★: `(seed, n, approx_chars)` 만의 함수. 본문 PRNG 는 제목과 다른 교란 시드라 제목/본문이
/// 독립적이다(제목만 회상하고 본문은 못 하는 상황을 분리 관측 가능).
/// ★content-filter 안전★: 모든 문장이 `SENTENCE_TEMPLATES` 를 채운 문법적 영어라 무해하다(모듈 헤더).
/// ★길이 근사★: 문단 단위로 쌓다가 목표를 넘으면 멈춘다 → 실제 길이는 목표 ± 마지막 문단.
pub fn filler_doc(seed: u64, n: u32, approx_chars: usize) -> String {
    // ★방어적 상한(finding 7)★: approx_chars 가 폭주 값(usize::MAX 등)이면 with_capacity 예약·문단 루프가
    //   즉시 OOM/panic 이다. CLI 가 이미 DOC_CHARS_CLAMP 로 클램프하지만, 이 순수 함수를 직접 부르는
    //   경로(테스트·미래 호출자)도 있으니 여기서도 하드 상한을 건다(cli::DOC_CHARS_CLAMP 와 동일 값).
    let approx_chars = approx_chars.min(super::cli::DOC_CHARS_CLAMP);
    let title = doc_title(seed, n);
    let mut out = String::with_capacity(approx_chars + 256);
    out.push_str(&format!("DOC-{n}: {title}\n"));

    // 본문 전용 PRNG(제목과 다른 교란) — 제목/본문 독립.
    let mut rng = Xorshift64::new(seed ^ (0x2545_F491_4F6C_DD1Du64.wrapping_mul(n as u64 + 1)));
    // 문단을 목표 길이 근사까지 쌓는다. 헤더 길이도 예산에 포함.
    while out.len() < approx_chars {
        out.push_str(&paragraph(&mut rng));
        out.push_str("\n\n");
    }
    out
}

/// 자연어 문단 1개 — 3~6개 문장. 각 문장은 템플릿을 결정적으로 채운다.
fn paragraph(rng: &mut Xorshift64) -> String {
    let sentences = 3 + rng.below(4); // 3..=6
    let mut p = String::new();
    for i in 0..sentences {
        if i > 0 {
            p.push(' ');
        }
        p.push_str(&fill_template(rng));
    }
    p
}

/// 템플릿 1개를 골라 슬롯을 benign 어휘로 채운다. 결과는 항상 문법적 영어 문장이다.
///
/// 지원 슬롯: `{place}` `{item}` `{cond}` `{action}` `{weather}` `{role}`. 첫 글자는 대문자로 시작하는
/// 템플릿(문두 슬롯 포함)이라 별도 대문자화가 필요 없다 — 단, `{role}` 이 문두인 템플릿은 role 어휘의
/// 첫 글자가 소문자("the ...")이므로 그 경우만 대문자화한다.
fn fill_template(rng: &mut Xorshift64) -> String {
    // ★슬롯 채우기 순서 고정★: 템플릿 선택 → 각 슬롯 타입을 정해진 순서로 뽑는다. 결정성을 위해
    //   슬롯 등장 순서가 아니라 **슬롯 타입별 고정 순서**로 뽑으면 안 된다(문장마다 슬롯 조합이 달라
    //   PRNG 소비량이 흔들려 재현성이 깨진다). 그래서 템플릿 문자열을 왼→오로 스캔하며 등장하는
    //   슬롯마다 순차로 뽑는다(등장 순서 = 소비 순서 고정).
    let template = rng.pick(SENTENCE_TEMPLATES);
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = match after.find('}') {
            Some(c) => c,
            None => {
                // 닫는 중괄호 없음(있을 수 없는 템플릿) — 방어적으로 남은 문자열 그대로.
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
            _ => "", // 미지 슬롯(있을 수 없음) — 빈 문자열로 제거.
        };
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    // 문두가 소문자면(예: "the day shift team ...") 첫 글자만 대문자화 — 문법성 보정.
    capitalize_first(&out)
}

/// 문자열의 첫 ASCII 알파벳 글자를 대문자화(이미 대문자면 그대로). 자연어 문장 시작 보정.
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
        // seed 0 은 SplitMix 상수로 대체돼 0 고정점을 피해야 한다.
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
        // 세 토큰 형식(형용사 + 명사 + 명사).
        assert_eq!(t1.split(' ').count(), 3, "제목은 3 토큰: {t1:?}");
    }

    #[test]
    fn doc_titles_differ_across_n() {
        // n 이 다르면 (거의 항상) 제목이 달라야 한다 — 최소한 인접 몇 개가 전부 동일하진 않게.
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
        // 헤더 뒤에 개행이 온다.
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
        // 목표 근사: 최소 approx 이상(루프가 넘을 때까지 쌓음), 한 문단(+헤더) 정도의 초과만 허용.
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
        // ★finding 7★: approx_chars=usize::MAX 여도 OOM/panic 없이 DOC_CHARS_CLAMP 근사로 끝나야 한다.
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
        // ASCII 자연어라 항상 유효 UTF-8 이고 문장 마침표를 포함한다.
        let doc = filler_doc(3, 4, 1000);
        assert!(doc.is_ascii(), "ASCII 만 사용(멀티바이트 이슈 회피)");
        assert!(doc.contains('.'), "문장 마침표 존재");
    }

    /// ★no gibberish(content-filter 안전 근거)★: 본문의 모든 문장이 템플릿 중 하나를 채운 형태여야 한다.
    /// 각 템플릿을 정규식-free 매처(고정 접두/접미)로 검사한다 — 슬롯 자리를 와일드카드로 보고 나머지
    /// 리터럴 조각이 순서대로 등장하는지 확인. 하나라도 안 맞으면 "gibberish"(무의미 난열) 로 간주해 실패.
    #[test]
    fn every_sentence_matches_a_template() {
        // 여러 seed·doc 로 다양한 문장을 수집해 검사(단일 표본 우연 통과 방지).
        let mut sentences: Vec<String> = Vec::new();
        for seed in [1u64, 42, 0xABCD, 7] {
            for n in 1..=3u32 {
                let doc = filler_doc(seed, n, 2500);
                // 헤더(첫 줄)를 제외한 본문을 문장 단위로 쪼갠다. 문단은 빈 줄로 구분, 문장은 ". " 로 근사 분해.
                for para in doc.lines().skip(1) {
                    for raw in para.split(". ") {
                        let s = raw.trim();
                        if s.is_empty() {
                            continue;
                        }
                        // 마지막 문장은 마침표를 포함할 수 있으니 정규화(끝 마침표 제거해 통일).
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

    /// 문장이 어느 템플릿의 **리터럴 조각 순서**를 만족하는지(슬롯은 와일드카드). 템플릿을 `{...}` 기준
    /// 리터럴 조각으로 쪼갠 뒤, 그 조각들이 문장에 **순서대로** 등장하면 그 템플릿에 부합한다고 본다.
    fn sentence_matches_any_template(sentence: &str) -> bool {
        // 문두 대문자화·끝 마침표 제거를 문장 쪽에 이미 적용했으므로, 템플릿도 동일 정규화한다.
        let sent_lower = sentence.to_ascii_lowercase();
        SENTENCE_TEMPLATES.iter().any(|t| {
            let t = t.trim_end_matches('.').to_ascii_lowercase();
            // 리터럴 조각(슬롯 사이 텍스트)만 뽑아 순서 매칭.
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
                            // 공백뿐인 리터럴은 스킵(슬롯 인접).
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
            // 템플릿 꼬리 리터럴.
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
