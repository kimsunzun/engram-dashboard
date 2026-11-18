//! Deterministic filler document generator (ADR-0090 Stage 2 pilot).
//!
//! ## Role
//! Makes natural-language prose documents for context saturation deterministically (seed fixed): the same seed · doc number · target length gives a byte-for-byte identical document every run (reproducibility pin = ADR-0088 d5a).
//!
//! ## ★Content-filter safety★
//! The initial xorshift pseudo-prose (random words listed from a vocabulary) was refused by claude's content filter as "violates Usage Policy" — a meaningless token run is presumed to read as a suspicious signal. Replacement: template-based natural language (logistics reports · weather logs · facility inspection notes — grammatical English sentences).
//!
//! ## Core invariants
//! - Determinism: the only random source is one xorshift64 seeded by the seed — no external state, clock or env.
//! - Grammatical natural language: every body sentence is a `SENTENCE_TEMPLATES` entry with slots filled — content-filter safety; the "no gibberish" unit test verifies it.
//! - Approximate length: `approx_chars` is a target, cut at paragraph boundaries (±one paragraph). The saturation loop uses the cumulative characters we sent as the progress signal, so this suffices.

/// Inline xorshift64 PRNG (SplitMix-family constants) — statistical quality is not required, only diversity of slot selection.
pub struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    /// Seeded by the seed. xorshift64 has a fixed point at 0, so seed==0 is replaced with a SplitMix constant to stay non-zero.
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

    /// The n==0 branch is defensive — callers pass only non-zero.
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

// ── benign vocabulary slots ──────────────────────────────────────────────────────────────────

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

/// Separated from the body vocabulary so the title is visually distinct.
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

/// The deterministic title of doc n — a function of `(seed, n)` alone; the driver calls it to reconstruct the probe answer (doc1_title_recalled), so the derivation rule exists only here (single source).
///
/// Three tokens: moderately hard for claude to recall exactly (smearing it into "the document" fails), and grading is clear by exact match.
pub fn doc_title(seed: u64, n: u32) -> String {
    let mut rng = Xorshift64::new(seed ^ (0xD1B5_4A32_D192_ED03u64.wrapping_mul(n as u64 + 1)));
    let adj = rng.pick(TITLE_ADJ);
    let noun1 = rng.pick(TITLE_NOUN);
    let noun2 = rng.pick(TITLE_NOUN);
    format!("{adj} {noun1} {noun2}")
}

/// The complete body of doc n — the `DOC-<n>: <title>\n` header + natural-language paragraphs.
///
/// The body PRNG uses a perturbed seed different from the title's, so title and body are independent (recalling only the title can be observed separately).
pub fn filler_doc(seed: u64, n: u32, approx_chars: usize) -> String {
    // ★Defensive cap★: a runaway approx_chars (usize::MAX etc.) makes the with_capacity reservation and the paragraph loop an immediate OOM/panic. The CLI already clamps, but tests and future callers can call this pure function directly, so cap here too.
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

/// A template starting with `{role}` needs capitalization because role vocabulary starts lowercase ("the ...").
fn fill_template(rng: &mut Xorshift64) -> String {
    // ★Fixed slot-filling order★: do not draw in a fixed order per slot type (slot combinations differ per sentence, so PRNG consumption varies and reproducibility breaks). Scan the template left→right and draw per slot as it appears (appearance order = fixed consumption order).
    let template = rng.pick(SENTENCE_TEMPLATES);
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = match after.find('}') {
            Some(c) => c,
            None => {
                // No closing brace — defense against a template that cannot exist.
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
            _ => "", // Unknown slot — cannot happen.
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
            assert_eq!(
                a.next_u64(),
                b.next_u64(),
                "the same seed gives the same sequence"
            );
        }
    }

    #[test]
    fn xorshift_seed_zero_is_nonzero_stream() {
        let mut r = Xorshift64::new(0);
        assert_ne!(
            r.next_u64(),
            0,
            "seed 0 must not become a zero sequence (fixed-point avoidance)"
        );
    }

    #[test]
    fn doc_title_is_deterministic() {
        let t1 = doc_title(7, 3);
        let t2 = doc_title(7, 3);
        assert_eq!(t1, t2, "same (seed, n) → same title");
        assert_eq!(t1.split(' ').count(), 3, "a title is 3 tokens: {t1:?}");
    }

    #[test]
    fn doc_titles_differ_across_n() {
        let titles: Vec<String> = (1..=10).map(|n| doc_title(99, n)).collect();
        let distinct: std::collections::HashSet<_> = titles.iter().collect();
        assert!(
            distinct.len() >= 5,
            "at least 5 of 10 doc titles must be distinct (variety): {titles:?}"
        );
    }

    #[test]
    fn filler_doc_header_format_and_title_match() {
        let doc = filler_doc(123, 1, 500);
        let title = doc_title(123, 1);
        let expected_header = format!("DOC-1: {title}");
        assert!(
            doc.starts_with(&expected_header),
            "the header must be `DOC-<n>: <title>` and the title must match doc_title: first line={:?}",
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
            "same (seed, n, approx_chars) → byte-identical (reproducibility pin)"
        );
    }

    #[test]
    fn filler_doc_approx_length_reached() {
        let approx = 4000;
        let doc = filler_doc(1, 2, approx);
        assert!(
            doc.len() >= approx,
            "reaches the minimum target length: {}",
            doc.len()
        );
        assert!(
            doc.len() < approx + 2000,
            "the overshoot must be one paragraph in size (not runaway): {}",
            doc.len()
        );
    }

    #[test]
    fn filler_docs_differ_across_n() {
        let d1 = filler_doc(5, 1, 2000);
        let d2 = filler_doc(5, 2, 2000);
        assert_ne!(d1, d2, "different doc numbers give different bodies");
    }

    #[test]
    fn filler_doc_clamps_absurd_length() {
        let doc = filler_doc(1, 1, usize::MAX);
        assert!(
            doc.len() < super::super::cli::DOC_CHARS_CLAMP + 4000,
            "runaway length is limited to the clamp + one paragraph: {}",
            doc.len()
        );
        assert!(
            doc.len() >= super::super::cli::DOC_CHARS_CLAMP,
            "fills up to the clamp"
        );
    }

    #[test]
    fn filler_doc_is_valid_utf8_prose() {
        let doc = filler_doc(3, 4, 1000);
        assert!(doc.is_ascii(), "ASCII only (avoids multibyte issues)");
        assert!(doc.contains('.'), "sentence periods present");
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
        assert!(sentences.len() > 30, "enough samples: {}", sentences.len());
        for s in &sentences {
            assert!(
                sentence_matches_any_template(s),
                "gibberish sentence (matches no template): {s:?}"
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
